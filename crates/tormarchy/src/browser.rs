//! Browser-only mode: a browser in a systemd scope of its own, which the
//! firewall lets reach nothing but loopback before the browser even starts.
//!
//! A proxy preference is a request that WebRTC and QUIC happily ignore. The
//! lock is an nftables rule matching the scope's cgroup, added by root from
//! inside the scope; if it cannot go on, the browser never starts.

use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use tormarchy_core::host::{Host, System, SOCKS_PORT};
use tormarchy_core::{ops, sys, Fail, Res};

/// Every browser worth supporting, with the family that decides how it is told
/// to use a proxy. Anything Chromium-based takes the same flags, anything
/// Firefox-based needs prefs, and a browser not listed still gets the generic
/// path.
const BROWSERS: [(&str, &str); 22] = [
    ("firefox", "firefox"),
    ("librewolf", "firefox"),
    ("waterfox", "firefox"),
    ("floorp", "firefox"),
    ("zen-browser", "firefox"),
    ("icecat", "firefox"),
    ("chromium", "chromium"),
    ("google-chrome-stable", "chromium"),
    ("google-chrome", "chromium"),
    ("brave", "chromium"),
    ("brave-browser", "chromium"),
    ("vivaldi-stable", "chromium"),
    ("vivaldi", "chromium"),
    ("microsoft-edge-stable", "chromium"),
    ("opera", "chromium"),
    ("thorium-browser", "chromium"),
    ("ungoogled-chromium", "chromium"),
    ("qutebrowser", "qutebrowser"),
    ("epiphany", "generic"),
    ("falkon", "generic"),
    ("midori", "generic"),
    ("luakit", "generic"),
];

fn family_of(name: &str) -> &'static str {
    BROWSERS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, f)| *f)
        .unwrap_or("generic")
}

fn installed() -> Vec<&'static str> {
    BROWSERS
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| sys::which_user(n).is_some())
        .collect()
}

fn default_browser() -> Option<String> {
    let first_word = |out: sys::Output| {
        out.stdout_str()
            .split_whitespace()
            .next()
            .map(str::to_string)
    };
    if let Some(helper) = sys::which_user("omarchy-default-browser") {
        let helper = helper.display().to_string();
        if let Some(b) = first_word(sys::Run::new(&helper, Vec::<String>::new()).run()) {
            if sys::which_user(&b).is_some() {
                return Some(b);
            }
        }
    }
    let desktop = sys::Run::new("xdg-settings", ["get", "default-web-browser"])
        .run()
        .stdout_str();
    let name = desktop
        .trim()
        .trim_end_matches(".desktop")
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_string();
    (!name.is_empty() && sys::which_user(&name).is_some()).then_some(name)
}

/// A Firefox profile of Tormarchy's own, with the proxy and the obvious leak
/// sources off. Separate rather than editing theirs: this browsing should share
/// nothing with their normal session, and their profile must not stay proxied.
fn firefox_profile(dir: &Path) -> Res {
    fs::create_dir_all(dir)
        .map_err(|e| Fail(format!("could not create {}: {e}", dir.display())))?;
    let prefs = format!(
        r#"// Managed by tormarchy. Regenerated on every launch.
user_pref("network.proxy.type", 1);
user_pref("network.proxy.socks", "127.0.0.1");
user_pref("network.proxy.socks_port", {SOCKS_PORT});
user_pref("network.proxy.socks_version", 5);
// Hostnames resolved by Tor, not locally.
user_pref("network.proxy.socks_remote_dns", true);
user_pref("network.proxy.allow_hijacking_localhost", false);
// WebRTC reports local and public addresses by design, the classic bypass.
user_pref("media.peerconnection.enabled", false);
// DNS over HTTPS would resolve through Firefox's own resolver, not Tor.
user_pref("network.trr.mode", 5);
user_pref("network.captive-portal-service.enabled", false);
user_pref("network.connectivity-service.enabled", false);
"#
    );
    fs::write(dir.join("user.js"), prefs).map_err(|e| Fail(e.to_string()))
}

pub fn launch(args: &[String]) -> Res {
    if args.first().map(String::as_str) == Some("--list") {
        let found = installed();
        if found.is_empty() {
            println!("  no supported browser found on PATH");
        }
        for name in found {
            println!("  {name:<24} {}", family_of(name));
        }
        return Ok(());
    }
    // A browser run as root is a root process reading the web. Never.
    if sys::is_root() {
        return Err(Fail("launch the browser as yourself, not as root".into()));
    }
    if sys::which("systemd-run").is_none() {
        return Err(Fail("systemd-run is required for browser-only mode".into()));
    }
    if !System::new(tormarchy_core::paths::Paths::system()).tor_running() {
        return Err(Fail(
            "tor is not running; connect first (browser-only mode still needs the SOCKS proxy)"
                .into(),
        ));
    }

    let (chosen, extra) = match args.first() {
        Some(name) => {
            sys::which_user(name)
                .ok_or_else(|| Fail(format!("no such browser on PATH: {name}")))?;
            (name.clone(), &args[1..])
        }
        None => {
            let b = default_browser()
                .or_else(|| installed().first().map(|s| s.to_string()))
                .ok_or("no supported browser found; install one or name it explicitly")?;
            (b, &args[0..0])
        }
    };

    // Each family gets an instance of its own, never the one already open: a
    // running browser handed a window by command line opens it in the existing
    // process, which is neither proxied nor locked down.
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or("HOME is not set")?
        .join("tormarchy");
    let name = chosen.rsplit('/').next().unwrap_or(&chosen).to_string();
    // By its full path: what runs inside the scope finds programs on this
    // helper's fixed PATH, which a browser in ~/.local/bin is not on.
    let exe = sys::which_user(&chosen)
        .ok_or_else(|| Fail(format!("no such browser on PATH: {chosen}")))?
        .display()
        .to_string();
    let mkdir = |d: &Path| {
        fs::create_dir_all(d).map_err(|e| Fail(format!("could not create {}: {e}", d.display())))
    };
    let mut cmd: Vec<String> = match family_of(&name) {
        "firefox" => {
            let dir = data.join("firefox");
            firefox_profile(&dir)?;
            vec![
                exe.clone(),
                "--profile".into(),
                dir.display().to_string(),
                "--new-instance".into(),
            ]
        }
        "chromium" => {
            let dir = data.join(format!("chromium-{name}"));
            mkdir(&dir)?;
            // host-resolver-rules stops Chromium resolving names itself;
            // without it DNS leaves in clear beside a proxied page.
            vec![
                exe.clone(),
                format!("--user-data-dir={}", dir.display()),
                format!("--proxy-server=socks5://127.0.0.1:{SOCKS_PORT}"),
                "--host-resolver-rules=MAP * ~NOTFOUND , EXCLUDE 127.0.0.1".into(),
                "--proxy-bypass-list=<-loopback>".into(),
            ]
        }
        "qutebrowser" => {
            let dir = data.join("qutebrowser");
            mkdir(&dir)?;
            vec![
                exe.clone(),
                "--basedir".into(),
                dir.display().to_string(),
                "-s".into(),
                "content.proxy".into(),
                format!("socks://127.0.0.1:{SOCKS_PORT}"),
            ]
        }
        _ => {
            // These take only the proxy environment and have no flag known to
            // force a separate instance; if one is open, a new window would land
            // in it, outside the lockdown. Refuse rather than guess.
            let short: String = name.chars().take(15).collect();
            let uid = sys::euid().to_string();
            if sys::Run::new("pgrep", ["-u", &uid, "-x", &short])
                .run()
                .ok()
            {
                return Err(Fail(format!(
                    "{name} is already running. Close it first: a new window would open in that unproxied instance instead of a locked-down one."
                )));
            }
            vec![exe.clone()]
        }
    };
    cmd.extend(extra.iter().cloned());

    let tag: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "_.-".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    println!("Launching {name} with only Tor reachable...");
    let proxy = format!("socks5h://127.0.0.1:{SOCKS_PORT}");
    let me = std::env::current_exe().map_err(|e| Fail(e.to_string()))?;
    let err = Command::new("systemd-run")
        .args(["--user", "--scope", "--collect", "--quiet"])
        .arg(format!(
            "--unit=tormarchy-browser-{tag}-{}",
            std::process::id()
        ))
        .arg(format!("--setenv=ALL_PROXY={proxy}"))
        .arg(format!("--setenv=all_proxy={proxy}"))
        .arg(format!("--setenv=http_proxy={proxy}"))
        .arg(format!("--setenv=https_proxy={proxy}"))
        .arg("--setenv=no_proxy=127.0.0.1,::1,localhost")
        .arg("--")
        .arg(me)
        .arg("browser-exec")
        .arg("--")
        .args(&cmd)
        .exec();
    Err(Fail(format!("could not run systemd-run: {err}")))
}

/// Inside the scope systemd-run just made: lock it, then become the browser.
/// The unprivileged half of a launch and nothing else -- as root this would
/// start whatever it was handed as root.
pub fn exec(args: &[String]) -> Res {
    if sys::is_root() {
        return Err(Fail("browser-exec never runs as root".into()));
    }
    let args = if args.first().map(String::as_str) == Some("--") {
        &args[1..]
    } else {
        args
    };
    let Some((program, rest)) = args.split_first() else {
        return Err(Fail("browser-exec: nothing to launch".into()));
    };
    let me = std::env::current_exe().map_err(|e| Fail(e.to_string()))?;
    // A child, not an exec: root reads the scope from this process, its parent.
    let locked = Command::new(me)
        .arg("browser-lock")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !locked {
        return Err(Fail(
            "could not lock the browser to Tor, so it was not started. Nothing was launched."
                .into(),
        ));
    }
    let err = Command::new(program).args(rest).exec();
    Err(Fail(format!("could not start {program}: {err}")))
}

/// Root. The scope is read from the caller's own process -- never taken as an
/// argument -- and must belong to the invoking user.
pub fn lock(h: &System) -> Res {
    if sys::which("nft").is_none() {
        return Err(Fail("nft is not installed".into()));
    }
    let user = h
        .invoking_user
        .clone()
        .ok_or("could not tell which user is launching the browser")?;
    let (uid, _, _) = sys::user_info(&user).ok_or_else(|| Fail(format!("unknown user {user}")))?;
    // SAFETY: no preconditions.
    let ppid = unsafe { libc::getppid() };
    let cgroups = sys::read_bounded_string(&h.paths.proc_root.join(format!("{ppid}/cgroup")), 4096)
        .unwrap_or_default();
    let cgroup = cgroups
        .lines()
        .find_map(|l| l.strip_prefix("0::"))
        .unwrap_or("")
        .to_string();
    ops::browser_lock(h, &cgroup, uid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families() {
        assert_eq!(family_of("librewolf"), "firefox");
        assert_eq!(family_of("brave"), "chromium");
        assert_eq!(family_of("qutebrowser"), "qutebrowser");
        assert_eq!(family_of("something-else"), "generic");
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn a_browser_on_the_callers_own_path_is_found_by_full_path() {
        let dir = sys::create_temp_dir("tm-browser-").unwrap();
        let fake = dir.join("tormarchy-test-browser");
        std::fs::write(&fake, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        // As main does: the caller's PATH remembered, then replaced.
        std::env::set_var("PATH", format!("{}:{}", dir.display(), sys::FIXED_PATH));
        sys::remember_user_path();
        std::env::set_var("PATH", sys::FIXED_PATH);
        assert_eq!(
            sys::which_user("tormarchy-test-browser"),
            Some(fake.clone())
        );
        assert_eq!(
            sys::which("tormarchy-test-browser"),
            None,
            "root's lookups never see it"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
