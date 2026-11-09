//! Putting the system side in place and taking it away again, symmetrically.
//!
//! The files setup installs are compiled into this binary, so the one signed
//! artifact the bootstrap verified is everything that lands as root -- nothing
//! is read from the user-writable plugin checkout.

use std::fs;
use std::path::Path;
use std::time::Duration;

use tormarchy_core::firewall::{self, NFT_BROWSER_TABLE, NFT_TABLE};
use tormarchy_core::host::{Host, System};
use tormarchy_core::paths::{Paths, HELPER_PATH};
use tormarchy_core::sys::{self, Run};
use tormarchy_core::{bridges, state, Fail, Res};

const TORRC_SNIPPET: &str = include_str!("../../../etc/torrc.d/00-tormarchy.conf");
const TMPFILES: &str = include_str!("../../../etc/tmpfiles/tor.conf");
const POLKIT_POLICY: &str = include_str!("../../../etc/polkit/com.tormarchy.policy");
const POLKIT_RULE: &str = include_str!("../../../etc/polkit/49-tormarchy.rules.in");
const BASH_COMPLETION: &str = include_str!("../../../etc/completion/tormarchy.bash");
const ZSH_COMPLETION: &str = include_str!("../../../etc/completion/_tormarchy");

const INCLUDE_MARKER: &str = "# Added by tormarchy setup.";
const POLKIT_ACTION_COUNT: usize = 7;

fn step(s: &str) {
    println!("\n\x1b[1m{s}\x1b[0m");
}

fn install_file(path: &Path, content: &[u8], mode: u32) -> Res {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .map_err(|e| Fail(format!("could not create {}: {e}", dir.display())))?;
    }
    sys::write_atomic(path, content, mode)
        .map_err(|e| Fail(format!("could not write {}: {e}", path.display())))
}

fn sudo_user() -> Option<String> {
    std::env::var("SUDO_USER")
        .ok()
        .filter(|u| sys::valid_user_name(u) && u != "root")
}

/// Run as the person who ran sudo, in their own home. Root writing into a home
/// directory follows whatever symlinks the user has there.
pub fn menu_as_user(user: &str, verb: &str) -> Res<String> {
    let (_, _, home) = sys::user_info(user).ok_or_else(|| Fail(format!("unknown user {user}")))?;
    if !home.is_dir() {
        return Err(Fail(format!("{user} has no home directory")));
    }
    let helper = if Path::new(HELPER_PATH).exists() {
        HELPER_PATH.to_string()
    } else {
        std::env::current_exe()
            .map_err(|e| Fail(e.to_string()))?
            .display()
            .to_string()
    };
    let home_env = format!("HOME={}", home.display());
    let out = Run::new(
        "runuser",
        ["-u", user, "--", "env", &home_env, &helper, "menu", verb],
    )
    .timeout(Duration::from_secs(30))
    .run();
    if out.ok() {
        Ok(out.stdout_str().trim().to_string())
    } else {
        Err(Fail(
            out.stderr_str()
                .trim()
                .trim_start_matches("tormarchy: ")
                .to_string(),
        ))
    }
}

/// `sudo tormarchy setup`, from the directory the bootstrap verified and
/// unpacked: this binary and tormarchyd beside it.
pub fn setup(_args: &[String]) -> Res {
    let paths = Paths::system();
    let host = System::new(paths.clone());
    let user = sudo_user();
    let me = std::env::current_exe().map_err(|e| Fail(e.to_string()))?;
    let daemon = me.with_file_name("tormarchyd");
    if !daemon.is_file() {
        return Err(Fail(format!(
            "{} is missing beside this binary; run setup from the plugin, which fetches both",
            daemon.display()
        )));
    }

    step("Installing tor");
    if host.tor_installed() {
        println!("  already installed");
    } else {
        // No -y: refreshing the database without upgrading is the partial
        // upgrade Arch warns against.
        let ok = std::process::Command::new("pacman")
            .args(["-S", "--needed", "--noconfirm", "tor"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return Err(Fail("could not install tor. If pacman reported 404s, the package database is out of date: run 'omarchy update', then setup again.".into()));
        }
    }

    step(&format!(
        "Installing tormarchy into {}",
        paths.bin_dir.display()
    ));
    for (src, name) in [(&me, "tormarchy"), (&daemon, "tormarchyd")] {
        let bytes =
            fs::read(src).map_err(|e| Fail(format!("could not read {}: {e}", src.display())))?;
        install_file(&paths.bin_dir.join(name), &bytes, 0o755)?;
        println!("  {}", paths.bin_dir.join(name).display());
    }
    // The old fifteen-executable layout, and its library.
    if let Ok(dir) = fs::read_dir(&paths.bin_dir) {
        for e in dir.flatten() {
            if e.file_name().to_string_lossy().starts_with("tormarchy-") {
                sys::remove_file_quiet(&e.path());
            }
        }
    }
    let _ = fs::remove_dir_all("/usr/local/lib/tormarchy");

    step("Configuring tor");
    install_file(&paths.torrc_base, TORRC_SNIPPET.as_bytes(), 0o644)?;
    println!("  {}", paths.torrc_base.display());
    // The directory form: tor reads every file in it in lexical order, and
    // naming a file that does not exist yet is a hard config error.
    let torrc = fs::read_to_string(&paths.torrc).unwrap_or_default();
    let dir = paths.torrc_dir.display().to_string();
    let included = torrc.lines().any(|l| {
        let mut f = l.split_whitespace();
        f.next() == Some("%include")
            && f.next()
                .map(|d| d.trim_end_matches('/') == dir)
                .unwrap_or(false)
            && f.next().is_none()
    });
    if included {
        println!("  {} already includes {dir}", paths.torrc.display());
    } else {
        let mut next = torrc.clone();
        if !next.is_empty() && !next.ends_with('\n') {
            next.push('\n');
        }
        next.push_str(&format!("\n{INCLUDE_MARKER}\n%include {dir}\n"));
        install_file(&paths.torrc, next.as_bytes(), 0o644)?;
        println!("  added the include to {}", paths.torrc.display());
    }

    // tor ships DataDirectory as 0700, which stops the tor group reaching the
    // group-readable cookie. A tmpfiles override, not a chmod: the packaged
    // rule is reapplied at boot and would quietly put 0700 back.
    install_file(
        &paths.tmpfiles_dir.join("tor.conf"),
        TMPFILES.as_bytes(),
        0o644,
    )?;
    Run::new(
        "systemd-tmpfiles",
        [
            "--create",
            &paths.tmpfiles_dir.join("tor.conf").display().to_string(),
        ],
    )
    .run();
    println!(
        "  {} (/var/lib/tor is now 0750)",
        paths.tmpfiles_dir.join("tor.conf").display()
    );

    let obsolete = Path::new("/etc/systemd/system/tor.service.d/10-tormarchy.conf");
    if obsolete.exists() {
        sys::remove_file_quiet(obsolete);
        let _ = fs::remove_dir("/etc/systemd/system/tor.service.d");
        host.systemctl(&["daemon-reload"]);
        println!("  removed an obsolete systemd drop-in");
    }
    if Run::new("systemctl", ["is-failed", "tor.service"])
        .run()
        .ok()
    {
        host.systemctl(&["reset-failed", "tor.service"]);
        println!("  cleared a previous failed start");
    }

    step("Installing shell completion");
    if paths
        .bash_completion
        .parent()
        .map(Path::is_dir)
        .unwrap_or(false)
    {
        install_file(
            &paths.bash_completion.join("tormarchy"),
            BASH_COMPLETION.as_bytes(),
            0o644,
        )?;
        println!("  {}", paths.bash_completion.join("tormarchy").display());
    }
    if Path::new("/usr/share/zsh").is_dir() {
        install_file(
            &paths.zsh_completion.join("_tormarchy"),
            ZSH_COMPLETION.as_bytes(),
            0o644,
        )?;
        println!("  {}", paths.zsh_completion.join("_tormarchy").display());
    }

    step("Installing the polkit policy");
    install_file(
        &paths.polkit_actions.join("com.tormarchy.policy"),
        POLKIT_POLICY.as_bytes(),
        0o644,
    )?;
    println!(
        "  {}",
        paths.polkit_actions.join("com.tormarchy.policy").display()
    );
    match &user {
        Some(u) => {
            install_file(
                &paths.polkit_rules.join("49-tormarchy.rules"),
                POLKIT_RULE.replace("@TORMARCHY_USER@", u).as_bytes(),
                0o644,
            )?;
            println!(
                "  {} (granted to {u})",
                paths.polkit_rules.join("49-tormarchy.rules").display()
            );
        }
        None => eprintln!(
            "  SKIPPED the rule: could not tell who invoked sudo, so it cannot name a user."
        ),
    }

    step("Verifying polkit registered the action");
    // The install can succeed and still leave the action unregistered, so ask
    // polkitd rather than trusting the file copy.
    if sys::which("pkaction").is_some() {
        let n = Run::new("pkaction", Vec::<String>::new())
            .run()
            .stdout_str()
            .lines()
            .filter(|l| l.starts_with("com.tormarchy."))
            .count();
        if n == POLKIT_ACTION_COUNT {
            println!("  {n} actions registered");
        } else {
            eprintln!("  WARNING: {n} of {POLKIT_ACTION_COUNT} actions registered; some commands will prompt for a password");
        }
    }

    step("Verifying the tor config");
    host.tor_verify().map_err(|why| {
        Fail(format!(
            "tor rejected the configuration ({why}); nothing has been started"
        ))
    })?;
    println!("  config is valid");

    step("Control port access");
    if let Some(u) = &user {
        if sys::user_groups(u).iter().any(|g| g == "tor") {
            println!("  {u} is already in the 'tor' group");
        } else if Run::new("usermod", ["-aG", "tor", u]).run().ok() {
            println!("  added {u} to the 'tor' group");
        } else {
            return Err(Fail(format!("could not add {u} to the tor group")));
        }
    }

    // 0711 lets the panel reach the summary by name; the stash stays 0600.
    if let Some(dir) = paths.bridge_stash.parent().filter(|d| d.is_dir()) {
        step("Updating saved bridges for the panel");
        let _ = sys::ensure_dir(dir, 0o711);
        if bridges::write_summary(&paths).is_ok() {
            println!("  {}", paths.bridge_summary.display());
        }
    }

    if let Some(u) = &user {
        step("Adding Tor to the Omarchy menu");
        match menu_as_user(u, "add") {
            Ok(msg) => println!("  {msg}"),
            Err(_) => println!("  could not; add it later with: tormarchy menu add"),
        }
    }

    // Deliberately not enabled at boot: tor starting itself every boot would be
    // a surprise, and the ruleset is applied by connect, never at boot -- which
    // guarantees a reboot always restores normal networking.
    println!(
        "
  tor is installed but NOT enabled at boot, on purpose: a reboot always
  restores normal networking, which is the way out of a wedged ruleset.

  Connecting will not ask for a password. Delete
  /etc/polkit-1/rules.d/49-tormarchy.rules to go back to being asked.

  Next:
    Click the onion in the bar, or run:  tormarchy connect
    Check for leaks with:                tormarchy doctor
    Emergency reset:                     tormarchy panic"
    );
    Ok(())
}

pub fn uninstall(purge: bool) -> Res {
    let paths = Paths::system();
    let host = System::new(paths.clone());

    // FIRST, before anything else is removed: taking this command away while a
    // ruleset is loaded would leave a machine with no network and nothing
    // installed that knows how to give it back.
    step("Removing any firewall rules");
    match firewall::delete_table(&host, NFT_TABLE) {
        Ok(true) => println!("  removed the {NFT_TABLE} table"),
        Ok(false) => println!("  none were loaded"),
        Err(e) => eprintln!("  FAILED to remove the {NFT_TABLE} table: {e}"),
    }
    if let Ok(true) = firewall::delete_table(&host, NFT_BROWSER_TABLE) {
        println!("  removed the {NFT_BROWSER_TABLE} table");
    }
    firewall::ipv6_restore(&host);
    state::clear_state(&paths);
    for f in [
        &paths.lock_file,
        &paths.panic_marker,
        &paths.browser_lock_file,
    ] {
        sys::remove_file_quiet(f);
    }

    // Before the binary goes: a boot unit left behind would try to reconnect
    // on the next boot with nothing installed to do it.
    step("Removing the boot receiver");
    if paths.boot_unit.exists() {
        host.systemctl(&["disable", "tormarchy-boot.service"]);
        sys::remove_file_quiet(&paths.boot_unit);
        host.systemctl(&["daemon-reload"]);
        println!("  removed and disabled");
    } else {
        println!("  was not installed");
    }
    let _ = fs::remove_dir_all(&paths.persist_dir);

    // Stopped, not disabled. Setup never enables tor, so whether it starts at
    // boot is a choice made before this was installed.
    step("Stopping tor");
    host.systemctl(&["stop", "tor.service"]);
    host.systemctl(&["reset-failed", "tor.service"]);
    println!("  stopped (whether it starts at boot is left as it was)");

    if let Some(u) = sudo_user() {
        step("Removing Tor from the Omarchy menu");
        match menu_as_user(&u, "remove") {
            Ok(msg) => println!("  {msg}"),
            Err(_) => println!("  nothing to remove"),
        }
    }

    step("Removing files");
    for f in [
        paths.helper(),
        paths.daemon(),
        paths.torrc_base.clone(),
        paths.torrc_exit.clone(),
        paths.torrc_exclude.clone(),
        paths.torrc_bridges.clone(),
        paths.polkit_actions.join("com.tormarchy.policy"),
        paths.polkit_rules.join("49-tormarchy.rules"),
        paths.bash_completion.join("tormarchy"),
        paths.zsh_completion.join("_tormarchy"),
    ] {
        sys::remove_file_quiet(&f);
    }
    println!("  helpers, torrc snippets, polkit files and completion");

    // Only our own include, recognised by the marker above it: the tor package
    // may ship that same line.
    if let Ok(torrc) = fs::read_to_string(&paths.torrc) {
        if torrc.lines().any(|l| l == INCLUDE_MARKER) {
            let mut out = String::new();
            let mut lines = torrc.split_inclusive('\n').peekable();
            while let Some(l) = lines.next() {
                if l.trim_end() == INCLUDE_MARKER {
                    if lines
                        .peek()
                        .map(|n| n.trim_end() == format!("%include {}", paths.torrc_dir.display()))
                        .unwrap_or(false)
                    {
                        lines.next();
                    }
                    if out.ends_with("\n\n") {
                        out.pop();
                    }
                    continue;
                }
                out.push_str(l);
            }
            install_file(&paths.torrc, out.as_bytes(), 0o644)?;
            println!("  removed our include from {}", paths.torrc.display());
        }
    }

    let tmpfiles = paths.tmpfiles_dir.join("tor.conf");
    if tmpfiles.exists() {
        sys::remove_file_quiet(&tmpfiles);
        Run::new(
            "systemd-tmpfiles",
            ["--create", "/usr/lib/tmpfiles.d/tor.conf"],
        )
        .run();
        println!("  restored /var/lib/tor to its packaged mode");
    }

    if purge {
        step("Purging");
        if let Some(u) = sudo_user() {
            if Run::new("gpasswd", ["-d", &u, "tor"]).run().ok() {
                println!("  removed {u} from the 'tor' group");
            }
        }
        let _ = fs::remove_dir_all("/etc/tormarchy");
        if Run::new("pacman", ["-Rns", "--noconfirm", "tor"])
            .timeout(Duration::from_secs(300))
            .run()
            .ok()
        {
            println!("  removed the tor package");
        } else {
            println!("  left the tor package (in use or already gone)");
        }
    }

    println!(
        "
  Tormarchy's system side is removed and networking is untouched.
  The plugin itself is still in the bar; remove it with:

    omarchy plugin remove tripleu.tor"
    );
    Ok(())
}

/// The unit `boot enable` installs. Opt-in for a reason: nothing else here
/// persists across a reboot, so restarting has always been the guaranteed way
/// out of a wedged firewall. connect applies rules only after tor reports a
/// full bootstrap, so with no network at boot the machine comes up normally.
fn boot_unit(paths: &Paths) -> String {
    format!(
        "# Managed by tormarchy. Installed by `tormarchy boot enable`, removed by disable.

[Unit]
Description=Reconnect Tormarchy if Tor was on at shutdown
Documentation=https://github.com/TripleU613/tormarchy
After=network-online.target nss-lookup.target
Wants=network-online.target
ConditionPathExists={marker}

[Service]
Type=oneshot
RemainAfterExit=no
ExecStart={helper} connect
TimeoutStartSec=180
SuccessExitStatus=0 1

[Install]
WantedBy=multi-user.target
",
        marker = paths.reconnect_marker.display(),
        helper = paths.helper().display()
    )
}

pub fn boot(verb: &str) -> Res {
    let paths = Paths::system();
    let host = System::new(paths.clone());
    match verb {
        "" | "status" => {
            if paths.boot_unit.exists() {
                let enabled = Run::new("systemctl", ["is-enabled", "tormarchy-boot.service"]).run().stdout_str().trim().to_string();
                println!("Boot reconnect: installed ({})", if enabled.is_empty() { "unknown" } else { &enabled });
                match state::remembered_mode(&paths) {
                    Some(m) => println!("  Tor was on when last connected, so it will reconnect at boot ({m} mode)"),
                    None => println!("  not currently armed -- connect once and it will be"),
                }
            } else {
                println!("Boot reconnect: off\n  enable with: sudo tormarchy boot enable");
            }
        }
        "enable" => {
            crate::require_terminal_root()?;
            install_file(&paths.boot_unit, boot_unit(&paths).as_bytes(), 0o644)?;
            host.systemctl(&["daemon-reload"]);
            if !host.systemctl(&["enable", "tormarchy-boot.service"]) {
                return Err(Fail("installed the unit but could not enable it".into()));
            }
            // Armed now if Tor is already on.
            if state::rules_present(&host) {
                if let Some(m) = state::current_mode(&paths) {
                    state::remember_connected(&paths, m);
                }
            }
            println!(
                "Boot reconnect is on.

At boot, if Tor was on when the machine went down, it reconnects. If the network
is unreachable or Tor cannot bootstrap, no rules are applied and the machine
comes up with ordinary networking -- connect never applies a ruleset it has not
verified.

Worth knowing what this costs: nothing else here survives a reboot, so
restarting has been the guaranteed way out of a wedged firewall. This trades a
little of that for convenience. Turn it back off with:

  sudo tormarchy boot disable"
            );
        }
        "disable" => {
            crate::require_terminal_root()?;
            host.systemctl(&["disable", "tormarchy-boot.service"]);
            sys::remove_file_quiet(&paths.boot_unit);
            host.systemctl(&["daemon-reload"]);
            state::forget_connected(&paths);
            println!("Boot reconnect is off. A reboot always comes up with normal networking.");
        }
        _ => println!(
            "Usage: tormarchy boot <status|enable|disable>\n\nReconnects at boot if Tor was on when the machine went down. Off by default,\nbecause a reboot is otherwise the guaranteed way out of a wedged firewall."
        ),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_policy_has_one_action_per_pkexec_command() {
        assert_eq!(
            POLKIT_POLICY.matches("<action id=\"com.tormarchy.").count(),
            POLKIT_ACTION_COUNT
        );
        for c in crate::PKEXEC_COMMANDS {
            assert!(
                POLKIT_POLICY.contains(&format!("com.tormarchy.{c}\"")),
                "no action for {c}"
            );
        }
        assert!(POLKIT_POLICY.contains(HELPER_PATH));
    }

    #[test]
    fn the_boot_unit_runs_the_installed_helper() {
        let u = boot_unit(&Paths::system());
        assert!(u.contains("ExecStart=/usr/local/bin/tormarchy connect"));
        assert!(u.contains("ConditionPathExists=/var/lib/tormarchy/reconnect"));
    }
}
