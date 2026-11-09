//! The commands that change routing: connect, disconnect, panic, mode, and the
//! browser lockdown. Each decides here and acts through a [`Host`], so the
//! ordering guarantees below are tested against a fake that misbehaves on cue.

use std::time::Duration;

use crate::bridges;
use crate::firewall::{self, NFT_BROWSER_TABLE};
use crate::host::Host;
use crate::socks::PROBE_URLS;
use crate::state::{self, Lock};
use crate::{Fail, Mode, Res};

const LOCK_WAIT: Duration = Duration::from_secs(30);
const BOOTSTRAP_WAIT: Duration = Duration::from_secs(120);
/// A route faster than this is taken at once; slower, fresh circuits are tried.
const ROUTE_GOOD_MS: u64 = 3500;
const ROUTE_ATTEMPTS: usize = 3;

/// A disconnect that ran while connect was waiting stopped tor: nothing more to
/// do then, and certainly no bridges to start it back up again.
fn cancelled_unless_running(host: &dyn Host) -> Res {
    if host.tor_running() {
        Ok(())
    } else {
        Err(Fail(
            "cancelled: tor was stopped before it finished connecting. No rules applied.".into(),
        ))
    }
}

/// Bootstrap progress over the wait, stopping early when tor goes away -- a
/// disconnect cancelling this connect, not a slow network.
fn wait_bootstrap(host: &dyn Host, wait: Duration) -> u8 {
    let deadline = host.now() + wait;
    let mut pct = 0;
    while host.now() < deadline {
        if !host.tor_running() {
            break;
        }
        pct = host.bootstrap().unwrap_or(0);
        if pct >= 100 {
            break;
        }
        host.sleep(Duration::from_secs(1));
    }
    pct
}

fn wait_circuit(host: &dyn Host, wait: Duration) {
    let deadline = host.now() + wait;
    while host.now() < deadline {
        if !host.tor_running() || host.circuit_established() {
            return;
        }
        host.sleep(Duration::from_secs(1));
    }
}

/// Bootstrapped is not the same as carrying traffic. Before any rule goes on,
/// something real is fetched through Tor and timed; if nothing answers, or the
/// answer is slow, fresh circuits are asked for and it tries again. Moving the
/// machine onto a dead or crawling route is the network drop; onto one already
/// proven, a blip. The DNS lookup comes first because every app's first request
/// is one, and tor gives each listening port circuits of its own.
pub fn find_route(host: &dyn Host) -> Res<u64> {
    wait_circuit(host, Duration::from_secs(20));
    let mut best: Option<u64> = None;
    for attempt in 0..ROUTE_ATTEMPTS {
        cancelled_unless_running(host)?;
        let target = PROBE_URLS[attempt % PROBE_URLS.len()];
        if !host.dns_probe() {
            host.say("No answer from Tor's resolver yet; trying another route...");
        } else if let Some(ms) = host.route_probe_ms(target) {
            best = Some(best.map_or(ms, |b| b.min(ms)));
            if ms <= ROUTE_GOOD_MS {
                host.say(&format!("Route found ({ms} ms)."));
                return Ok(ms);
            }
            host.say(&format!("Route is slow ({ms} ms); trying another..."));
        } else {
            host.say("Nothing answered through Tor yet; trying another route...");
        }
        if attempt + 1 < ROUTE_ATTEMPTS {
            let _ = host.newnym();
            host.sleep(Duration::from_secs(2));
        }
    }
    // Slow is still a working route, and better than not connecting at all.
    match best {
        Some(ms) => {
            host.say(&format!("Using the fastest route found ({ms} ms)."));
            Ok(ms)
        }
        None => Err(Fail(format!(
            "nothing got through Tor in {ROUTE_ATTEMPTS} tries. No rules applied, networking is unchanged."
        ))),
    }
}

/// Called once rules are on. Panic writes its token before it flushes, so
/// either the token is already new here, or panic's flush runs after these
/// rules went on and removes them itself. Either way no rules are left.
fn undo_if_panicked(host: &dyn Host, before: &str) -> Res {
    if state::panic_token(host.paths()) == before {
        return Ok(());
    }
    firewall::flush_rules(host);
    firewall::ipv6_restore(host);
    state::clear_state(host.paths());
    state::forget_connected(host.paths());
    Err(Fail(
        "cancelled: panic ran while the rules were going on, and they are off again".into(),
    ))
}

pub fn connect(host: &dyn Host, mode: Mode) -> Res<String> {
    let paths = host.paths();
    let panic_before = state::panic_token(paths);
    if !host.tor_installed() {
        return Err(Fail("tor is not installed -- run setup first".into()));
    }
    if !paths.torrc_base.exists() {
        return Err(Fail(format!(
            "{} is missing -- run setup first",
            paths.torrc_base.display()
        )));
    }
    host.start_tor().map_err(Fail)?;
    if !host.publish_cookie() {
        host.warn("note: could not publish a readable control cookie; live stats may be empty");
    }

    if mode == Mode::Socks {
        let _lock = Lock::take(paths, LOCK_WAIT)?;
        firewall::flush_rules(host);
        firewall::ipv6_restore(host);
        state::write_state(paths, Mode::Socks, false)?;
        state::forget_connected(paths);
        return Ok(format!(
            "Tor is running. SOCKS proxy on 127.0.0.1:{}. Nothing is routed\nsystem-wide, so the rest of the machine is untouched.\n\n\
Launch a browser that can reach nothing but Tor:\n  tormarchy browser              # your default browser\n  tormarchy browser --list       # every browser found on this machine",
            crate::host::SOCKS_PORT
        ));
    }

    // Said before the wait, while there is still time to act on it.
    let live = host.live_apps();
    if !live.is_empty() {
        host.say("\nThese programs have connections open outside Tor right now:");
        for app in &live {
            host.say(&format!("  {app}"));
        }
        host.say(
            "\nThose connections were made as your real address and will be cut when the rules\n\
go on. Anything that reconnects afterwards goes through Tor, but a program that\n\
was already logged in stays logged in as the same account -- Tor changes where\n\
your traffic comes from, not who a website already knows you are.\n",
        );
    }

    host.say("Waiting for Tor to bootstrap...");
    let mut pct = wait_bootstrap(host, BOOTSTRAP_WAIT);
    cancelled_unless_running(host)?;

    // A stall looks the same whether the network is slow or blocking Tor
    // outright, and a bridge answers both, so try rather than making the user
    // diagnose censorship from a percentage.
    if pct < 100 && paths.bridge_stash.exists() && !paths.torrc_bridges.exists() {
        host.say(&format!(
            "Stalled at {pct}%. This network may be blocking Tor; retrying through bridges..."
        ));
        let stash = crate::sys::read_bounded_string(&paths.bridge_stash, bridges::INPUT_LIMIT)
            .unwrap_or_default();
        if bridges::activate(host, &stash).is_ok() {
            let _ = bridges::reload(host);
            pct = wait_bootstrap(host, BOOTSTRAP_WAIT);
            cancelled_unless_running(host)?;
            if pct >= 100 {
                host.say("Bridges worked. Leaving them on for this network.");
            } else {
                // Put back as found, so the next attempt on a normal network is
                // not silently handicapped by bridges turned on here.
                crate::sys::remove_file_quiet(&paths.torrc_bridges);
                host.systemctl(&["restart", "tor.service"]);
                host.publish_cookie();
                host.warn("Bridges did not help either; turned them back off.");
            }
        }
    }

    if pct < 100 {
        // No rules applied, so the network still works. tor is left running in
        // case it is merely slow; it is harmless without a ruleset.
        let mut hint = String::from("No rules applied, networking is unchanged.");
        if !paths.bridge_stash.exists() {
            hint.push_str(" If this network blocks Tor, add bridges: tormarchy bridge set");
        }
        return Err(Fail(format!("tor stalled at {pct}% bootstrap. {hint}")));
    }

    host.say("Finding a route...");
    find_route(host)?;

    let _lock = Lock::take(paths, LOCK_WAIT)?;
    // Checked again under the lock: a disconnect that got in between has
    // stopped tor, and rules applied now would shut the network with nothing
    // behind them.
    cancelled_unless_running(host)?;
    firewall::apply_rules(host, mode).map_err(|why| {
        Fail(format!(
            "could not apply the nftables ruleset ({why}). Networking is unchanged."
        ))
    })?;
    // After the rules, not before: if they fail to apply the machine is left as
    // found, and a disabled IPv6 stack would not be that.
    if firewall::ipv6_disable(host).is_err() {
        host.warn("note: could not turn off the IPv6 stack; the ruleset still drops IPv6");
    }
    state::write_state(paths, mode, true)?;
    state::remember_connected(paths, mode);
    undo_if_panicked(host, &panic_before)?;
    Ok(format!(
        "Connected. All traffic now routes through Tor ({mode} mode)."
    ))
}

pub fn disconnect(host: &dyn Host) -> Res<String> {
    let paths = host.paths();
    let _lock = Lock::take(paths, LOCK_WAIT)?;
    // Rules first, then the daemon. The other order leaves a window where
    // traffic is redirected to a Tor no longer listening: a dead network, but
    // failing closed rather than leaking.
    firewall::flush_rules(host);
    firewall::ipv6_restore(host);
    host.systemctl(&["stop", "tor.service"]);
    state::clear_state(paths);
    // Deliberate disconnect means do not come back on the next boot.
    state::forget_connected(paths);
    Ok("Disconnected from Tor.".into())
}

/// The escape hatch. Deliberately takes no lock: it must never wait behind
/// anything. Its token, written before anything is flushed, tells a connect or
/// mode change applying rules right now to take them back off.
pub fn panic(host: &dyn Host) -> Res<String> {
    let paths = host.paths();
    if !state::mark_panic(paths) {
        host.warn("note: could not mark the panic for a connect in flight");
    }
    firewall::flush_rules(host);
    firewall::ipv6_restore(host);
    // tor too, without waiting on it: a connect still bootstrapping checks tor
    // is running before it applies anything.
    host.systemctl(&["stop", "--no-block", "tor.service"]);
    state::clear_state(paths);
    // Someone reaching for panic does not want the rules back at the next boot.
    state::forget_connected(paths);
    Ok("Removed all Tormarchy rules. Networking is back to normal.".into())
}

/// Changing mode never changes whether Tormarchy is on: a switch that is on
/// stays on in the new mode, and one that is off stays off.
pub fn set_mode(host: &dyn Host, mode: Mode) -> Res<String> {
    let paths = host.paths();
    let panic_before = state::panic_token(paths);
    // Before reading what is on, not after: a disconnect landing in between
    // left this applying a ruleset to a machine whose tor had just stopped.
    let lock = Lock::take(paths, LOCK_WAIT)?;
    let was_routed = state::rules_present(host);
    let browser_on = state::current_mode(paths) == Some(Mode::Socks) && host.tor_running();

    if mode == Mode::Socks {
        firewall::flush_rules(host);
        firewall::ipv6_restore(host);
        state::write_state(paths, Mode::Socks, false)?;
        // Boot reconnect is for the routed modes; browser-only never resumes.
        state::forget_connected(paths);
        return Ok(if was_routed || browser_on {
            "Switched to browser-only mode. System-wide routing is off; Tor is still running."
                .into()
        } else {
            "Mode set to browser-only. Connect to start Tor.".into()
        });
    }

    if was_routed {
        firewall::apply_rules(host, mode).map_err(|why| {
            Fail(format!(
                "could not apply the {mode} ruleset ({why}); previous rules left in place"
            ))
        })?;
        let _ = firewall::ipv6_disable(host);
        state::write_state(paths, mode, true)?;
        state::remember_connected(paths, mode);
        undo_if_panicked(host, &panic_before)?;
        Ok(format!("Switched to {mode} mode."))
    } else if browser_on {
        // Browser-only was on, so this stays on: in the new mode that means its
        // rules, which go on through connect so they still wait for a full
        // bootstrap. connect takes the lock itself once tor has bootstrapped;
        // held across that wait it would shut out the disconnect that cancels.
        drop(lock);
        connect(host, mode)
    } else {
        state::write_state(paths, mode, false)?;
        Ok(format!("Mode set to {mode}. Connect to apply it."))
    }
}

/// Root. Confines the calling browser scope to loopback.
///
/// The scope is read from the caller's own process rather than taken as an
/// argument, and has to be one of the invoking user's tormarchy-browser scopes,
/// so this can only ever restrict the browser that asked.
///
/// A rule matches a cgroup by id, and the kernel keeps it after the cgroup is
/// gone, so rules added one per launch only ever grew. Instead every call
/// rebuilds the set -- one rule for each browser scope that exists right now,
/// the caller's included -- in a single transaction, so a running browser is
/// never without its rule.
pub fn browser_lock(host: &dyn Host, caller_cgroup: &str, uid: u32) -> Res {
    let paths = host.paths();
    let cgroup = caller_cgroup.trim_start_matches('/');
    if cgroup.is_empty() {
        return Err(Fail(
            "browser lockdown needs the unified cgroup hierarchy (cgroup v2)".into(),
        ));
    }
    if !firewall::is_browser_scope(cgroup, Some(uid)) {
        return Err(Fail(format!(
            "refusing to lock {cgroup}: not a tormarchy browser scope belonging to this user"
        )));
    }
    let _lock = Lock::take_browser(paths)?;
    // nft refuses the whole set if any scope in it is gone by the time it
    // commits -- another browser closing in between -- so a refusal is retried
    // on a fresh look before it counts.
    for _ in 0..3 {
        let scopes = firewall::browser_scopes(&paths.cgroup_root, cgroup);
        if host
            .nft(&["-f", "-"], Some(&firewall::browser_ruleset(&scopes)))
            .ok()
        {
            return Ok(());
        }
    }
    Err(Fail(format!(
        "nft rejected the browser lockdown (it needs nftables socket cgroupv2 matching, kernel 5.13+); {NFT_BROWSER_TABLE} unchanged"
    )))
}

/// `bridge set`: lines from stdin, validated, saved and switched to.
pub fn bridge_set(host: &dyn Host, raw: &[u8]) -> Res<String> {
    let paths = host.paths();
    let lines =
        bridges::normalize(raw).map_err(|e| Fail(format!("{e}, and nothing was changed")))?;
    let (counts, invalid) = bridges::count(&lines);
    if lines.trim().is_empty() {
        return Err(Fail(
            "no input on stdin; pipe bridge lines in, e.g. tormarchy bridge set < bridges.txt"
                .into(),
        ));
    }
    if counts.is_empty() {
        return Err(Fail(
            "no bridge lines found; each line starts with a transport name, e.g. obfs4".into(),
        ));
    }
    if invalid > 0 {
        return Err(Fail(format!(
            "{invalid} line(s) are not bridge lines; nothing was changed"
        )));
    }
    // 0711: the panel can reach the summary by name without being able to list
    // the directory, and the stash inside stays 0600.
    let dir = paths.bridge_stash.parent().expect("stash has a directory");
    crate::sys::ensure_dir(dir, 0o711).map_err(|e| Fail(e.to_string()))?;
    bridges::activate(host, &lines)?;
    crate::sys::write_atomic(&paths.bridge_stash, lines.as_bytes(), 0o600)
        .map_err(|e| Fail(e.to_string()))?;
    let _ = bridges::write_summary(paths);
    bridges::reload(host)?;
    Ok("Bridges active.".into())
}

pub fn bridge_on(host: &dyn Host) -> Res<String> {
    let paths = host.paths();
    let stash = crate::sys::read_bounded_string(&paths.bridge_stash, bridges::INPUT_LIMIT)
        .ok_or("no stashed bridge lines; use 'tormarchy bridge set' first")?;
    bridges::activate(host, &stash)?;
    if let Some(dir) = paths.bridge_stash.parent() {
        let _ = crate::sys::ensure_dir(dir, 0o711);
    }
    let _ = bridges::write_summary(paths);
    bridges::reload(host)?;
    Ok("Bridges reactivated.".into())
}

pub fn bridge_off(host: &dyn Host) -> Res<String> {
    let paths = host.paths();
    crate::sys::remove_file_quiet(&paths.torrc_bridges);
    if let Some(dir) = paths.bridge_stash.parent() {
        if dir.exists() {
            let _ = crate::sys::ensure_dir(dir, 0o711);
        }
    }
    let _ = bridges::write_summary(paths);
    bridges::reload(host)?;
    Ok("Bridges off. Connecting directly.".into())
}

#[cfg(test)]
pub mod testing {
    //! A host made of scratch files and switches.
    use std::cell::{Cell, RefCell};
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use crate::host::Host;
    use crate::paths::Paths;
    use crate::sys::{self, Output};

    type Hook = Box<dyn Fn(&Fake)>;

    pub struct Fake {
        pub paths: Paths,
        pub root: PathBuf,
        pub running: Cell<bool>,
        pub rules_on: Cell<bool>,
        pub bootstrap: Cell<u8>,
        pub route_ms: RefCell<Vec<Option<u64>>>,
        pub dns_ok: Cell<bool>,
        pub nft_ok: Cell<bool>,
        pub log: RefCell<Vec<String>>,
        pub nft_inputs: RefCell<Vec<String>>,
        /// Called inside apply: lets a test land a panic mid-apply.
        pub on_apply: RefCell<Option<Hook>>,
        pub clock: Cell<Duration>,
        pub start: Instant,
        pub transports: RefCell<Vec<String>>,
        pub verify_ok: Cell<bool>,
    }

    impl Fake {
        pub fn new() -> Fake {
            let root = sys::create_temp_dir("tm-ops-").unwrap();
            let paths = Paths::under(&root);
            std::fs::create_dir_all(&paths.torrc_dir).unwrap();
            std::fs::create_dir_all(paths.lock_file.parent().unwrap()).unwrap();
            std::fs::write(&paths.torrc_base, "SocksPort 9050\n").unwrap();
            for iface in ["all", "default", "lo", "eth0"] {
                std::fs::create_dir_all(paths.ipv6_conf.join(iface)).unwrap();
                std::fs::write(paths.ipv6_conf.join(iface).join("disable_ipv6"), "0").unwrap();
            }
            Fake {
                paths,
                root,
                running: Cell::new(false),
                rules_on: Cell::new(false),
                bootstrap: Cell::new(100),
                route_ms: RefCell::new(vec![Some(500)]),
                dns_ok: Cell::new(true),
                nft_ok: Cell::new(true),
                log: RefCell::new(vec![]),
                nft_inputs: RefCell::new(vec![]),
                on_apply: RefCell::new(None),
                clock: Cell::new(Duration::ZERO),
                start: Instant::now(),
                transports: RefCell::new(vec!["obfs4".into()]),
                verify_ok: Cell::new(true),
            }
        }
        pub fn did(&self, what: &str) -> bool {
            self.log.borrow().iter().any(|l| l.contains(what))
        }
    }

    impl Default for Fake {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    impl Host for Fake {
        fn paths(&self) -> &Paths {
            &self.paths
        }
        fn tor_running(&self) -> bool {
            self.running.get()
        }
        fn systemctl(&self, args: &[&str]) -> bool {
            self.log
                .borrow_mut()
                .push(format!("systemctl {}", args.join(" ")));
            match args {
                ["stop", ..] => self.running.set(false),
                ["start", ..] | ["restart", ..] => self.running.set(true),
                _ => {}
            }
            true
        }
        fn nft(&self, args: &[&str], stdin: Option<&str>) -> Output {
            let input = stdin.unwrap_or("").to_string();
            self.nft_inputs.borrow_mut().push(input.clone());
            let ok = self.nft_ok.get();
            match args {
                ["-f", "-"]
                    if input.contains("delete table inet tormarchy\n")
                        && !input.contains("chain nat_output") =>
                {
                    self.rules_on.set(false);
                    self.log.borrow_mut().push("flush".into());
                }
                ["-f", "-"] if input.contains("chain nat_output") && ok => {
                    if let Some(f) = self.on_apply.borrow().as_ref() {
                        f(self);
                    }
                    self.rules_on.set(true);
                    self.log.borrow_mut().push("apply".into());
                }
                ["list", "table", "inet", "tormarchy"] => {
                    return Output {
                        status: Some(if self.rules_on.get() { 0 } else { 1 }),
                        ..Default::default()
                    }
                }
                _ => {}
            }
            Output {
                status: Some(if ok { 0 } else { 1 }),
                ..Default::default()
            }
        }
        fn tor_verify(&self) -> Result<(), String> {
            if self.verify_ok.get() {
                Ok(())
            } else {
                Err("Bridge line did not parse.".into())
            }
        }
        fn tor_installed(&self) -> bool {
            true
        }
        fn transport_binary(&self, t: &str) -> Option<PathBuf> {
            self.transports
                .borrow()
                .iter()
                .any(|x| x == t)
                .then(|| PathBuf::from(format!("/usr/bin/{t}-client")))
        }
        fn bootstrap(&self) -> Option<u8> {
            self.running.get().then(|| self.bootstrap.get())
        }
        fn circuit_established(&self) -> bool {
            true
        }
        fn newnym(&self) -> Result<(), String> {
            self.log.borrow_mut().push("newnym".into());
            Ok(())
        }
        fn dns_probe(&self) -> bool {
            self.dns_ok.get()
        }
        fn route_probe_ms(&self, _: &str) -> Option<u64> {
            let mut r = self.route_ms.borrow_mut();
            if r.len() > 1 {
                r.remove(0)
            } else {
                r.first().copied().flatten()
            }
        }
        fn publish_cookie(&self) -> bool {
            true
        }
        fn start_tor(&self) -> Result<(), String> {
            self.running.set(true);
            Ok(())
        }
        fn live_apps(&self) -> Vec<String> {
            vec![]
        }
        fn sleep(&self, d: Duration) {
            self.clock.set(self.clock.get() + d);
        }
        fn now(&self) -> Instant {
            self.start + self.clock.get()
        }
        fn say(&self, line: &str) {
            self.log.borrow_mut().push(format!("say {line}"));
        }
        fn warn(&self, line: &str) {
            self.log.borrow_mut().push(format!("warn {line}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::Fake;
    use super::*;
    use crate::state;

    // The lock is root-only, and these run unprivileged as often as not; the
    // logic under test does not depend on it.

    #[test]
    fn connect_applies_rules_only_after_a_proven_route() {
        let h = Fake::new();
        connect(&h, Mode::Strict).unwrap();
        assert!(h.rules_on.get());
        assert_eq!(state::current_mode(&h.paths), Some(Mode::Strict));
        assert!(h.paths.active_file.exists());
        assert_eq!(
            std::fs::read_to_string(h.paths.ipv6_conf.join("all/disable_ipv6")).unwrap(),
            "1"
        );
        assert_eq!(
            std::fs::read_to_string(h.paths.ipv6_conf.join("lo/disable_ipv6")).unwrap(),
            "0",
            "loopback v6 stays up"
        );
    }

    #[test]
    fn a_stalled_bootstrap_applies_nothing() {
        let h = Fake::new();
        h.bootstrap.set(40);
        let err = connect(&h, Mode::Lan).unwrap_err();
        assert!(err.0.contains("stalled at 40%"), "{err}");
        assert!(!h.rules_on.get());
        assert!(!h.did("apply"));
    }

    #[test]
    fn no_route_at_all_applies_nothing() {
        let h = Fake::new();
        *h.route_ms.borrow_mut() = vec![None];
        assert!(connect(&h, Mode::Lan).is_err());
        assert!(!h.did("apply"));
    }

    #[test]
    fn a_slow_route_is_retried_with_fresh_circuits_then_the_fastest_used() {
        let h = Fake::new();
        h.running.set(true);
        *h.route_ms.borrow_mut() = vec![Some(7000), Some(5200), Some(9000)];
        assert_eq!(find_route(&h).unwrap(), 5200);
        assert_eq!(h.log.borrow().iter().filter(|l| *l == "newnym").count(), 2);
        let h = Fake::new();
        h.running.set(true);
        *h.route_ms.borrow_mut() = vec![Some(6100), Some(900)];
        assert_eq!(find_route(&h).unwrap(), 900);
    }

    #[test]
    fn a_dead_resolver_is_no_route() {
        let h = Fake::new();
        h.running.set(true);
        h.dns_ok.set(false);
        let err = find_route(&h).unwrap_err();
        assert!(err.0.contains("nothing got through"), "{err}");
    }

    #[test]
    fn panic_landing_mid_apply_leaves_no_rules() {
        let h = Fake::new();
        *h.on_apply.borrow_mut() = Some(Box::new(|h: &Fake| {
            panic(h).unwrap();
        }));
        let err = connect(&h, Mode::Lan).unwrap_err();
        assert!(err.0.contains("panic ran"), "{err}");
        assert!(
            !h.rules_on.get(),
            "the rules connect put on after panic's flush are taken off"
        );
        assert!(!h.paths.active_file.exists());
        assert_eq!(
            std::fs::read_to_string(h.paths.ipv6_conf.join("all/disable_ipv6")).unwrap(),
            "0"
        );
    }

    #[test]
    fn a_mode_change_that_panic_overtook_fails_and_leaves_no_rules() {
        let h = Fake::new();
        connect(&h, Mode::Lan).unwrap();
        // The boot receiver is enabled, so the marker is real.
        std::fs::create_dir_all(h.paths.boot_unit.parent().unwrap()).unwrap();
        std::fs::write(&h.paths.boot_unit, "").unwrap();
        *h.on_apply.borrow_mut() = Some(Box::new(|h: &Fake| {
            panic(h).unwrap();
        }));
        if crate::sys::is_root() {
            assert!(set_mode(&h, Mode::Strict).is_err());
            assert!(!h.rules_on.get());
            assert!(
                !h.paths.reconnect_marker.exists(),
                "the boot reconnect panic cancelled stays cancelled"
            );
        }
        *h.on_apply.borrow_mut() = None;
    }

    #[test]
    fn panic_stops_a_connect_still_bootstrapping() {
        let h = Fake::new();
        h.running.set(true);
        panic(&h).unwrap();
        assert!(cancelled_unless_running(&h).is_err());
        assert!(h.did("systemctl stop --no-block tor.service"));
    }

    #[test]
    fn disconnect_takes_rules_down_before_stopping_tor() {
        let h = Fake::new();
        connect(&h, Mode::Lan).unwrap();
        disconnect(&h).unwrap();
        let log = h.log.borrow();
        let flush = log.iter().rposition(|l| l == "flush").unwrap();
        let stop = log
            .iter()
            .rposition(|l| l == "systemctl stop tor.service")
            .unwrap();
        assert!(flush < stop);
        assert!(!h.paths.state_dir.exists());
    }

    #[test]
    fn ipv6_is_put_back_exactly_once() {
        let h = Fake::new();
        std::fs::write(h.paths.ipv6_conf.join("eth0/disable_ipv6"), "1").unwrap();
        connect(&h, Mode::Lan).unwrap();
        // A second routed connect must not record the value it just wrote.
        connect(&h, Mode::Strict).unwrap();
        disconnect(&h).unwrap();
        let read = |i: &str| {
            std::fs::read_to_string(h.paths.ipv6_conf.join(i).join("disable_ipv6")).unwrap()
        };
        assert_eq!(read("all"), "0");
        assert_eq!(
            read("eth0"),
            "1",
            "an interface switched off on purpose stays off"
        );
        // And with nothing saved, nothing is touched.
        std::fs::write(h.paths.ipv6_conf.join("all/disable_ipv6"), "1").unwrap();
        firewall::ipv6_restore(&h);
        assert_eq!(read("all"), "1");
    }

    #[test]
    fn browser_only_applies_no_rules_and_forgets_the_boot_marker() {
        let h = Fake::new();
        connect(&h, Mode::Socks).unwrap();
        assert!(!h.rules_on.get());
        assert_eq!(state::current_mode(&h.paths), Some(Mode::Socks));
        assert!(!h.paths.active_file.exists());
    }

    #[test]
    fn the_bridge_fallback_tries_saved_bridges_then_puts_them_back() {
        let h = Fake::new();
        std::fs::create_dir_all(h.paths.bridge_stash.parent().unwrap()).unwrap();
        std::fs::write(&h.paths.bridge_stash, "obfs4 192.0.2.1:443 AAAA cert=x\n").unwrap();
        h.bootstrap.set(30);
        assert!(connect(&h, Mode::Lan).is_err());
        assert!(
            h.did("systemctl restart tor.service"),
            "tor was restarted onto the bridges"
        );
        assert!(
            !h.paths.torrc_bridges.exists(),
            "bridges that did not help are turned back off"
        );
    }

    #[test]
    fn bridge_set_refuses_oversized_input_and_changes_nothing() {
        let h = Fake::new();
        let mut big = "obfs4 192.0.2.1:443 AAAA cert=x\n"
            .repeat(3000)
            .into_bytes();
        big.truncate(bridges::INPUT_LIMIT + 1);
        let err = bridge_set(&h, &big).unwrap_err();
        assert!(err.0.contains("nothing was changed"));
        assert!(!h.paths.bridge_stash.exists() && !h.paths.torrc_bridges.exists());
    }

    #[test]
    fn a_bridge_line_tor_refuses_puts_the_old_set_back() {
        let h = Fake::new();
        bridges::activate(&h, "obfs4 192.0.2.1:443 AAAA cert=x\n").unwrap();
        let before = std::fs::read_to_string(&h.paths.torrc_bridges).unwrap();
        h.verify_ok.set(false);
        let err = bridges::activate(&h, "obfs4 192.0.2.9:443 ABCD cert=x\n").unwrap_err();
        assert!(err.0.contains("Bridge line did not parse"));
        assert_eq!(
            std::fs::read_to_string(&h.paths.torrc_bridges).unwrap(),
            before
        );
        std::fs::remove_file(&h.paths.torrc_bridges).unwrap();
        assert!(bridges::activate(&h, "obfs4 192.0.2.9:443 ABCD cert=x\n").is_err());
        assert!(
            !h.paths.torrc_bridges.exists(),
            "with none before, none are left behind"
        );
    }

    #[test]
    fn a_missing_transport_is_refused_with_how_to_get_it() {
        let h = Fake::new();
        let err =
            bridges::activate(&h, "webtunnel [2001:db8::1]:443 CCCC url=https://x\n").unwrap_err();
        assert!(err.0.contains("webtunnel"));
    }

    #[test]
    fn browser_lock_refuses_anything_but_the_callers_own_browser_scope() {
        let h = Fake::new();
        assert!(browser_lock(
            &h,
            "/user.slice/user-1000.slice/user@1000.service/app.slice/firefox.scope",
            1000
        )
        .is_err());
        assert!(browser_lock(
            &h,
            "/user.slice/user-1001.slice/user@1001.service/app.slice/tormarchy-browser-x-1.scope",
            1000
        )
        .is_err());
        assert!(browser_lock(&h, "", 1000).is_err());
        let mine = "user.slice/user-1000.slice/user@1000.service/app.slice/tormarchy-browser-firefox-41.scope";
        std::fs::create_dir_all(h.paths.cgroup_root.join(mine)).unwrap();
        for _ in 0..5 {
            browser_lock(&h, &format!("/{mine}"), 1000).unwrap();
        }
        let last = h.nft_inputs.borrow().last().cloned().unwrap();
        assert_eq!(
            last.matches("jump lockdown").count(),
            1,
            "locking again never grows the set"
        );
        h.nft_ok.set(false);
        assert!(browser_lock(&h, &format!("/{mine}"), 1000).is_err());
    }
}
