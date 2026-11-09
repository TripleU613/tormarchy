//! The leak check: is this actually working? Every check is recorded once with
//! a stable id, then printed for a terminal or handed to the panel as JSON, so
//! the two can never disagree.

use std::fs;
use std::time::Duration;

use serde::Serialize;

use crate::firewall::NFT_TABLE;
use crate::host::{Host, SOCKS_PORT};
use crate::sys::{self, Run};
use crate::{state, Mode};

/// check.torproject.org's answer is a few dozen bytes. Its detail goes into
/// the shell that runs the bar, so a larger answer is a failed check rather
/// than something to pass along.
pub const CHECK_REPLY_LIMIT: usize = 4096;
const CHECK_URL: &str = "https://check.torproject.org/api/ip";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pass,
    Warn,
    Fail,
    Skip,
}

#[derive(Clone, Debug, Serialize)]
pub struct Check {
    pub id: String,
    pub group: String,
    pub status: Status,
    pub text: String,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub ok: bool,
    pub failures: usize,
    pub checked_at: String,
    pub mode: String,
    pub checks: Vec<Check>,
    #[serde(skip)]
    pub connected: bool,
    #[serde(skip)]
    pub tor_running: bool,
}

struct Builder {
    group: String,
    checks: Vec<Check>,
}

impl Builder {
    fn add(
        &mut self,
        id: &str,
        status: Status,
        text: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.checks.push(Check {
            id: id.into(),
            group: self.group.clone(),
            status,
            text: text.into(),
            detail: detail.into(),
        });
    }
}

/// check.torproject.org through the given curl arguments: whether it says Tor,
/// and the address it saw when that is shaped like one.
pub fn ask_check_torproject(extra: &[&str]) -> Option<(bool, String)> {
    let mut args: Vec<String> = vec!["-sf".into(), "-m".into(), "25".into()];
    args.extend(extra.iter().map(|s| s.to_string()));
    args.push(CHECK_URL.into());
    let out = Run::new("curl", &args)
        .timeout(Duration::from_secs(30))
        .limit(CHECK_REPLY_LIMIT)
        .run();
    if !out.ok() || out.overflowed {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let is_tor = v.get("IsTor").and_then(|b| b.as_bool()).unwrap_or(false);
    let ip = v.get("IP").and_then(|s| s.as_str()).unwrap_or("");
    let ip = if (2..=45).contains(&ip.len())
        && ip
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == '.' || c == ':')
    {
        ip.to_string()
    } else {
        String::new()
    };
    Some((is_tor, ip))
}

fn read_trim(path: &str) -> String {
    fs::read_to_string(path)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

pub fn run(host: &dyn Host) -> Report {
    let paths = host.paths();
    let mode = state::current_mode(paths);
    let connected = state::rules_present(host);
    let tor_running = host.tor_running();
    let mut b = Builder {
        group: String::new(),
        checks: Vec::new(),
    };

    b.group = "Tor daemon".into();
    if !host.tor_installed() {
        b.add("tor", Status::Fail, "tor is not installed", "");
    } else if !tor_running {
        // Stopped while disconnected is the correct idle state, not a fault:
        // a reboot always restores normal networking that way.
        if connected {
            b.add(
                "tor",
                Status::Fail,
                "rules are applied but tor.service is not running -- the network is blocked",
                "",
            );
        } else {
            b.add(
                "tor",
                Status::Skip,
                "tor is stopped, which is normal while disconnected",
                "",
            );
        }
    } else {
        b.add("tor", Status::Pass, "tor.service is running", "");
        match host.bootstrap() {
            Some(100) => b.add("bootstrap", Status::Pass, "bootstrapped (100%)", ""),
            other => b.add(
                "bootstrap",
                Status::Fail,
                format!("bootstrap stalled at {}%", other.unwrap_or(0)),
                "",
            ),
        }
        match paths
            .cookie_candidates()
            .into_iter()
            .find(|p| sys::read_bounded(p, 64).is_ok())
        {
            Some(p) => b.add(
                "control",
                Status::Pass,
                format!("control cookie readable at {}", p.display()),
                "",
            ),
            None => b.add(
                "control",
                Status::Fail,
                format!("cannot read the control cookie: {}", cookie_hint(host)),
                "",
            ),
        }
    }

    b.group = "Routing".into();
    if mode == Some(Mode::Socks) {
        // Nothing is routed system-wide; what browser-only does offer is the
        // proxy, so that is what is proven.
        if !tor_running {
            b.add(
                "proxy",
                Status::Skip,
                "browser-only mode with tor stopped: there is no proxy to check",
                "",
            );
        } else {
            let proxy = format!("127.0.0.1:{SOCKS_PORT}");
            match ask_check_torproject(&["--socks5-hostname", &proxy]) {
                Some((true, ip)) => b.add(
                    "proxy",
                    Status::Pass,
                    format!("the SOCKS proxy on 127.0.0.1:{SOCKS_PORT} leaves through Tor"),
                    if ip.is_empty() {
                        String::new()
                    } else {
                        format!("exit as seen from outside: {ip}")
                    },
                ),
                Some((false, _)) => b.add(
                    "proxy",
                    Status::Fail,
                    "check.torproject.org says the SOCKS proxy is NOT Tor",
                    "",
                ),
                None => b.add(
                    "proxy",
                    Status::Fail,
                    "could not reach check.torproject.org through the SOCKS proxy",
                    "",
                ),
            }
        }
        b.add(
            "routing",
            Status::Skip,
            "browser-only mode: nothing else is routed system-wide, by design",
            "",
        );
    } else if !connected {
        b.add(
            "routing",
            Status::Skip,
            "not connected, so there is nothing to leak",
            "",
        );
    } else {
        match ask_check_torproject(&[]) {
            Some((true, ip)) => b.add(
                "exit",
                Status::Pass,
                "check.torproject.org confirms this connection is Tor",
                if ip.is_empty() {
                    String::new()
                } else {
                    format!("exit as seen from outside: {ip}")
                },
            ),
            Some((false, _)) => b.add(
                "exit",
                Status::Fail,
                "check.torproject.org says this is NOT Tor -- traffic is escaping the redirect",
                "",
            ),
            None => b.add(
                "exit",
                Status::Fail,
                "could not reach check.torproject.org (no route, or Tor is not carrying traffic)",
                "",
            ),
        }
        // Success here is a leak.
        if Run::new(
            "curl",
            ["-6", "-sf", "-m", "8", "https://ipv6.icanhazip.com"],
        )
        .timeout(Duration::from_secs(12))
        .limit(256)
        .run()
        .ok()
        {
            b.add(
                "ipv6",
                Status::Fail,
                "IPv6 reached the internet directly -- this is a leak",
                "",
            );
        } else {
            b.add("ipv6", Status::Pass, "IPv6 is blocked", "");
        }
        if read_trim(&paths.ipv6_conf.join("all/disable_ipv6").to_string_lossy()) == "1" {
            b.add(
                "ipv6-stack",
                Status::Pass,
                "IPv6 stack is disabled in the kernel as well",
                "",
            );
        } else {
            b.add(
                "ipv6-stack",
                Status::Warn,
                "IPv6 is filtered but the kernel stack is still up",
                "",
            );
        }
        if Run::new("getent", ["hosts", "example.com"])
            .timeout(Duration::from_secs(20))
            .run()
            .ok()
        {
            b.add(
                "dns",
                Status::Pass,
                "DNS resolves (through Tor's DNSPort)",
                "",
            );
        } else {
            b.add("dns", Status::Fail, "DNS is not resolving", "");
        }
        // Forwarded traffic never passes the output hook, so none of the
        // output rules apply to it -- and it is invisible from here.
        if read_trim(
            &paths
                .proc_root
                .join("sys/net/ipv4/ip_forward")
                .to_string_lossy(),
        ) == "1"
        {
            if sys::is_root() {
                if host
                    .nft(
                        &["list", "chain", "inet", NFT_TABLE, "filter_forward"],
                        None,
                    )
                    .ok()
                {
                    b.add(
                        "forwarding",
                        Status::Pass,
                        "IP forwarding is on, but the forward chain is dropping it",
                        "",
                    );
                } else {
                    b.add("forwarding", Status::Fail, "IP forwarding is on and the forward chain is MISSING -- routed traffic is leaking", "");
                }
            } else {
                // Every ruleset this applies has the forward chain, so if rules
                // are on, it is on.
                b.add(
                    "forwarding",
                    Status::Pass,
                    "IP forwarding is on, but the applied ruleset always drops forwarded traffic",
                    "",
                );
            }
        } else {
            b.add(
                "forwarding",
                Status::Pass,
                "IP forwarding is off, so nothing routes through this machine",
                "",
            );
        }
    }

    b.group = "Clock".into();
    // Tor will not build circuits against a badly skewed clock, and the ruleset
    // drops NTP while connected -- so a long session can drift itself broken.
    let synced = Run::new("timedatectl", ["show", "-p", "NTPSynchronized", "--value"])
        .timeout(Duration::from_secs(10))
        .run();
    if synced.stdout_str().trim() == "yes" {
        b.add("clock", Status::Pass, "clock is NTP-synchronised", "");
    } else if connected {
        b.add("clock", Status::Warn, "clock is not synchronised and NTP is blocked while connected. Disconnect briefly to resync if Tor starts failing.", "");
    } else {
        b.add(
            "clock",
            Status::Warn,
            "clock is not NTP-synchronised; Tor needs an accurate clock",
            "",
        );
    }

    let failures = b.checks.iter().filter(|c| c.status == Status::Fail).count();
    Report {
        ok: failures == 0,
        failures,
        checked_at: sys::utc_timestamp(sys::now_unix()),
        mode: mode.map(|m| m.as_str().to_string()).unwrap_or_default(),
        checks: b.checks,
        connected,
        tor_running,
    }
}

/// Why the cookie is unreadable, in the user's terms. "Join the tor group" is
/// useless advice to someone already in it, which is the usual case.
pub fn cookie_hint(host: &dyn Host) -> String {
    let user = std::env::var("SUDO_USER")
        .ok()
        .or_else(|| std::env::var("USER").ok())
        .or_else(|| sys::user_name(sys::euid()))
        .unwrap_or_default();
    if !sys::group_exists("tor") {
        return "the tor group does not exist -- is tor installed?".into();
    }
    if !sys::user_groups(&user).iter().any(|g| g == "tor") {
        return format!("{user} is not in the 'tor' group. Run setup, then log out and back in.");
    }
    if host.tor_running() {
        "no readable cookie has been published yet. Reconnect and it will appear.".into()
    } else {
        "tor is not running, so there is no control cookie to read yet.".into()
    }
}

/// The terminal view.
pub fn render(r: &Report) -> String {
    let mut out = String::from("Tormarchy doctor\n\n");
    out.push_str(&format!(
        "State\n  mode: {}\n  rules applied: {}\n  tor running: {}\n\n",
        if r.mode.is_empty() { "none" } else { &r.mode },
        r.connected,
        r.tor_running
    ));
    let mut group = "";
    for c in &r.checks {
        if c.group != group {
            if !group.is_empty() {
                out.push('\n');
            }
            group = &c.group;
            out.push_str(group);
            out.push('\n');
        }
        let mark = match c.status {
            Status::Pass => "\x1b[32m✓\x1b[0m",
            Status::Fail => "\x1b[31m✗\x1b[0m",
            Status::Warn => "\x1b[33m!\x1b[0m",
            Status::Skip => "\x1b[90m-\x1b[0m",
        };
        out.push_str(&format!("  {mark} {}\n", c.text));
        if !c.detail.is_empty() {
            out.push_str(&format!("    {}\n", c.detail));
        }
    }
    out.push('\n');
    if r.failures == 0 {
        out.push_str("No problems found.\n");
    } else {
        out.push_str(&format!("{} problem(s) found.\n", r.failures));
    }
    out
}
