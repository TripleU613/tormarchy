//! tormarchy -- route traffic through Tor, from the Omarchy bar.
//!
//! The system side in one program: the commands that need root, which it gets
//! by re-running itself under sudo (with a terminal) or pkexec (without), and
//! the ones that do not. Under pkexec only the commands on [`PKEXEC_COMMANDS`]
//! run, whatever polkit decided: the grant setup installs is passwordless, so
//! anything running as the user who installed this can reach it.

mod browser;
mod install;

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::time::Duration;

use tormarchy_core::host::{Host, System, SOCKS_PORT};
use tormarchy_core::paths::Paths;
use tormarchy_core::{
    bridges, doctor, exits, firewall, menu, ops, socks, state, status, sys, tor, Fail, Mode, Res,
};

/// The subcommands that may run as root through polkit, and nothing else. The
/// polkit policy has one action per entry, each pinned to it as the first
/// argument. Each takes only validated values; none runs anything its caller
/// names.
const PKEXEC_COMMANDS: [&str; 7] = [
    "connect",
    "disconnect",
    "mode",
    "exit",
    "bridge",
    "panic",
    "browser-lock",
];

/// Every command the dispatcher knows, for the tests that hold it to the help
/// and the polkit policy.
#[cfg_attr(not(test), allow(dead_code))]
const SUBCOMMANDS: [&str; 19] = [
    "connect",
    "disconnect",
    "mode",
    "status",
    "ip",
    "newnym",
    "exit",
    "menu",
    "bridge",
    "browser",
    "browser-exec",
    "browser-lock",
    "speed",
    "pingd",
    "doctor",
    "panic",
    "boot",
    "setup",
    "uninstall",
];

fn main() -> ExitCode {
    // Closed by default; files that must be readable say so with their mode.
    // SAFETY: umask has no preconditions.
    unsafe {
        libc::umask(0o077);
    }
    // Every external program is found on a fixed PATH, never the caller's.
    std::env::set_var("PATH", sys::FIXED_PATH);

    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().cloned().unwrap_or_else(|| "help".into());
    let rest: Vec<String> = args.iter().skip(1).cloned().collect();

    if std::env::var_os("PKEXEC_UID").is_some() && !PKEXEC_COMMANDS.contains(&command.as_str()) {
        return failed(Fail(
            "that subcommand does not run as root through polkit".into(),
        ));
    }

    match run(&command, &rest, &args) {
        Ok(code) => code,
        Err(e) => failed(e),
    }
}

fn failed(e: Fail) -> ExitCode {
    eprintln!("tormarchy: {e}");
    ExitCode::from(1)
}

fn system() -> System {
    System::new(Paths::system())
}

/// This program, by its resolved path. Escalation re-runs exactly this file.
fn self_path() -> PathBuf {
    std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .unwrap_or_else(|_| PathBuf::from(tormarchy_core::paths::HELPER_PATH))
}

/// sudo when there is a terminal to type into, pkexec otherwise. From the
/// shell there is no terminal, so this raises Omarchy's own polkit dialog.
fn require_root(argv: &[String]) -> Res {
    if sys::is_root() {
        return Ok(());
    }
    let tty = unsafe { libc::isatty(0) } == 1;
    let runner = if tty { "sudo" } else { "pkexec" };
    let err = {
        use std::os::unix::process::CommandExt;
        Command::new(runner).arg(self_path()).args(argv).exec()
    };
    Err(Fail(format!("could not run {runner}: {err}")))
}

/// For setup, uninstall and boot. The polkit actions pin this program's path
/// and the subcommand, but a terminal and sudo are what keep the passwordless
/// grant scoped to the runtime commands.
fn require_terminal_root() -> Res {
    if std::env::var_os("PKEXEC_UID").is_some() {
        return Err(Fail(
            "run this from a terminal with sudo, not through polkit".into(),
        ));
    }
    if !sys::is_root() {
        return Err(Fail("run this with sudo".into()));
    }
    Ok(())
}

fn print(s: &str) {
    if !s.is_empty() {
        println!("{s}");
    }
}

fn run(command: &str, rest: &[String], argv: &[String]) -> Res<ExitCode> {
    let arg = |i: usize| rest.get(i).map(String::as_str).unwrap_or("");
    match command {
        "connect" => {
            let (dry, mode_arg) = if arg(0) == "--dry-run" {
                (true, arg(1))
            } else {
                (false, arg(0))
            };
            let paths = Paths::system();
            let mode = if mode_arg.is_empty() {
                state::current_mode(&paths)
                    .or_else(|| state::remembered_mode(&paths))
                    .unwrap_or(Mode::Lan)
            } else {
                Mode::parse(mode_arg).ok_or_else(|| {
                    Fail(format!(
                        "unknown mode '{mode_arg}' (expected: lan strict socks)"
                    ))
                })?
            };
            if dry {
                return dry_run(mode);
            }
            require_root(argv)?;
            print(&ops::connect(&system(), mode)?);
        }
        "disconnect" => {
            require_root(argv)?;
            print(&ops::disconnect(&system())?);
        }
        "panic" => {
            require_root(argv)?;
            print(&ops::panic(&system())?);
        }
        "mode" => {
            let mode = Mode::parse(arg(0)).ok_or_else(|| {
                Fail(format!(
                    "unknown mode '{}' (expected: lan strict socks)",
                    arg(0)
                ))
            })?;
            require_root(argv)?;
            print(&ops::set_mode(&system(), mode)?);
        }
        "status" => {
            let h = system();
            let ep = h.endpoint();
            let mut ask = tor::OneShot {
                endpoint: &ep,
                timeout: Duration::from_secs(5),
            };
            let facts = status::gather(&h, &mut ask);
            if arg(0) == "--json" {
                println!("{}", status::to_json(&facts));
            } else {
                println!("{}", facts.status_text());
            }
        }
        "ip" => ip(arg(0) == "--json")?,
        "newnym" => {
            let h = system();
            if !h.tor_running() {
                return Err(Fail("tor is not running".into()));
            }
            h.newnym()
                .map_err(|e| Fail(format!("{e}: {}", doctor::cookie_hint(&h))))?;
            println!("New circuit requested.");
        }
        "exit" => exit(rest, argv)?,
        "bridge" => return bridge(rest, argv),
        "browser" => browser::launch(rest)?,
        "browser-exec" => browser::exec(rest)?,
        "browser-lock" => {
            require_root(argv)?;
            browser::lock(&system())?;
        }
        "speed" => return speed(arg(0) == "--json"),
        "pingd" => pingd(arg(0)),
        "doctor" => {
            let report = doctor::run(&system());
            if arg(0) == "--json" {
                println!("{}", serde_json::to_string(&report).unwrap_or_default());
            } else {
                print!("{}", doctor::render(&report));
            }
            return Ok(if report.ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            });
        }
        "boot" => install::boot(arg(0))?,
        "menu" => menu_cmd(arg(0))?,
        "setup" => {
            require_terminal_root()?;
            install::setup(rest)?;
        }
        "uninstall" => {
            require_terminal_root()?;
            install::uninstall(arg(0) == "--purge")?;
        }
        "version" | "--version" | "-V" => println!("tormarchy {}", tormarchy_core::VERSION),
        "help" | "-h" | "--help" => print!("{}", help()),
        other => {
            eprint!("{}", help());
            return Err(Fail(format!("unknown command: {other}")));
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn dry_run(mode: Mode) -> Res<ExitCode> {
    let ruleset = firewall::build_ruleset(mode);
    println!("{ruleset}");
    if !sys::is_root() {
        eprintln!("# Syntax check needs root (nft cannot read the kernel cache otherwise).");
        eprintln!("# Re-run as: sudo tormarchy connect --dry-run {mode}");
        return Ok(ExitCode::SUCCESS);
    }
    let check = system().nft(&["-c", "-f", "-"], Some(&ruleset));
    if check.ok() {
        eprintln!("# nft accepts this ruleset. Nothing was applied.");
        Ok(ExitCode::SUCCESS)
    } else {
        eprint!("{}", check.stderr_str());
        Err(Fail(
            "nft rejected the ruleset (nothing was applied)".into(),
        ))
    }
}

fn ip(json: bool) -> Res {
    let h = system();
    if !h.tor_running() {
        return Err(Fail("tor is not running".into()));
    }
    let ep = h.endpoint();
    let mut ask = tor::OneShot {
        endpoint: &ep,
        timeout: Duration::from_secs(5),
    };
    let path = tor::current_path(&mut ask).ok().flatten().ok_or_else(|| {
        Fail(format!(
            "no general-purpose circuit built yet: {}",
            doctor::cookie_hint(&h)
        ))
    })?;
    let age = path.age_seconds(sys::now_unix());
    let ccs: Vec<&str> = path.hops.iter().map(|h| h.cc.as_str()).collect();
    if json {
        println!(
            "{}",
            serde_json::json!({"circuitId": path.circuit_id, "exitIp": path.exit_ip(), "path": ccs.join(","), "ageSec": age})
        );
        return Ok(());
    }
    println!("Exit address   {}", path.exit_ip());
    println!(
        "Path           {}",
        ccs.iter()
            .map(|c| c.to_uppercase())
            .collect::<Vec<_>>()
            .join(" -> ")
    );
    let roles: Vec<String> = (0..ccs.len())
        .map(|i| {
            // The last hop is the exit whatever the length: onion services use
            // longer paths.
            let r = if i + 1 == ccs.len() {
                "exit"
            } else {
                ["guard", "middle"].get(i).copied().unwrap_or("relay")
            };
            format!("{r:<6}")
        })
        .collect();
    println!("               {}", roles.join("    "));
    println!("Circuit age    {}m {}s", age / 60, age % 60);
    println!("Circuit id     {}", path.circuit_id);
    Ok(())
}

fn exit(rest: &[String], argv: &[String]) -> Res {
    let target = rest.first().map(String::as_str).unwrap_or("");
    match target {
        "--list" => {
            let h = system();
            if !h.tor_running() {
                return Err(Fail("tor is not running".into()));
            }
            for cc in tor::exit_countries(&h.endpoint()).map_err(Fail)? {
                println!("{cc}");
            }
            Ok(())
        }
        "--exclude" => {
            let codes = exits::parse_exclusions(rest.get(1).map(String::as_str).unwrap_or(""))?;
            require_root(argv)?;
            print(&exits::exclude(&system(), &codes)?);
            Ok(())
        }
        _ => {
            if target != "auto" && exits::valid_code(target).is_none() {
                return Err(Fail(
                    "expected a two-letter country code, 'auto', --list, or --exclude".into(),
                ));
            }
            require_root(argv)?;
            print(&exits::pin(&system(), target)?);
            Ok(())
        }
    }
}

fn read_stdin_bounded() -> Res<Vec<u8>> {
    let mut buf = Vec::new();
    io::stdin()
        .take(bridges::INPUT_LIMIT as u64 + 1)
        .read_to_end(&mut buf)
        .map_err(|e| Fail(format!("could not read stdin: {e}")))?;
    Ok(buf)
}

fn bridge(rest: &[String], argv: &[String]) -> Res<ExitCode> {
    let verb = rest.first().map(String::as_str).unwrap_or("status");
    let h = system();
    match verb {
        "status" => {
            let counts = bridges::saved_counts(&h.paths);
            if rest.get(1).map(String::as_str) == Some("--json") {
                let mut r = bridges::report(&h, &counts);
                r.state = Some(bridges::state(&h.paths).into());
                println!("{}", serde_json::to_string(&r).unwrap_or_default());
            } else if h.paths.torrc_bridges.exists() {
                // From the saved lines, which the live file is always written
                // from -- that file is tor config, whose first words are never
                // transports.
                println!("Bridges: ACTIVE");
                for (t, n) in &counts {
                    if t == "plain" {
                        println!("  {n} plain (no transport needed)");
                    } else {
                        let a = if h.transport_binary(t).is_some() { "available" } else { "MISSING" };
                        println!("  {n} {t} ({a})");
                    }
                }
            } else if h.paths.bridge_stash.exists() || h.paths.bridge_summary.exists() {
                println!("Bridges: off (lines stashed -- 'tormarchy bridge on' to use them)");
            } else {
                println!("Bridges: off (none configured)");
            }
        }
        "check" => {
            // Unprivileged and side-effect free: what a paste holds -- how many,
            // which transports, whether this machine can run them -- before
            // anything is applied. The lines never leave this process.
            let lines = bridges::normalize(&read_stdin_bounded()?)?;
            let (counts, invalid) = bridges::count(&lines);
            let mut r = bridges::report(&h, &counts);
            r.invalid_lines = Some(invalid);
            println!("{}", serde_json::to_string(&r).unwrap_or_default());
        }
        "transports" => {
            for t in ["obfs4", "meek_lite", "snowflake", "webtunnel"] {
                match h.transport_binary(t) {
                    Some(b) => println!("  {t:<12} {}", b.display()),
                    None => println!("  {t:<12} missing -- {}", bridges::install_hint(t)),
                }
            }
        }
        "set" => {
            // Content on stdin only. A root helper reading a path chosen by an
            // unprivileged caller is an arbitrary file read; pkexec and sudo
            // both pass stdin through, so escalating first and reading after
            // removes the whole class of problem.
            require_root(argv)?;
            let raw = read_stdin_bounded()?;
            print(&ops::bridge_set(&h, &raw)?);
        }
        "on" => {
            require_root(argv)?;
            print(&ops::bridge_on(&h)?);
        }
        "off" => {
            require_root(argv)?;
            print(&ops::bridge_off(&h)?);
        }
        _ => print!(
            "Usage: tormarchy bridge <status [--json]|set|check|on|off|transports>\n\n  \
set     Read bridge lines from stdin and activate them:\n            tormarchy bridge set < bridges.txt\n  \
check   Read bridge lines from stdin and report what they need, changing\n          nothing. Needs no root.\n\n\
Get current bridge lines from https://bridges.torproject.org/options, the\nTelegram bot @GetBridgesBot, or bridges@torproject.org.\n"
        ),
    }
    Ok(ExitCode::SUCCESS)
}

fn speed(json: bool) -> Res<ExitCode> {
    let h = system();
    let out = |ok: bool, ms: u64, err: &str| {
        if json {
            println!(
                "{}",
                serde_json::json!({"ok": ok, "latencyMs": ms, "error": err})
            );
        } else if ok {
            println!("{ms} ms");
        } else {
            eprintln!("{err}");
        }
    };
    if !h.tor_running() {
        out(false, 0, "tor is not running");
        return Ok(ExitCode::from(1));
    }
    let mut last = String::new();
    for url in socks::PROBE_URLS {
        match socks::time_request(SOCKS_PORT, url, Duration::from_secs(20)) {
            Ok(ms) => {
                out(true, ms, "");
                return Ok(ExitCode::SUCCESS);
            }
            Err(e) => last = e.to_string(),
        }
    }
    let msg = if last.contains("timed out") || last.contains("WouldBlock") {
        "timed out through the Tor SOCKS proxy".to_string()
    } else {
        format!("no response through the Tor SOCKS proxy on 127.0.0.1:{SOCKS_PORT}")
    };
    out(false, 0, &msg);
    Ok(ExitCode::from(1))
}

/// Live latency: one line of milliseconds per reading, until killed. One
/// connection held open and re-timed, so each reading is one round trip.
fn pingd(interval: &str) {
    let secs: u64 = interval.parse().unwrap_or(3).clamp(1, 60);
    let h = system();
    let mut stream: Option<socks::Stream> = None;
    loop {
        if !h.tor_running() {
            stream = None;
            std::thread::sleep(Duration::from_secs(secs));
            continue;
        }
        if stream.is_none() {
            stream =
                socks::Stream::open(SOCKS_PORT, socks::LATENCY_URL, Duration::from_secs(30)).ok();
        }
        if let Some(s) = stream.as_mut() {
            match s.time_head(Duration::from_secs(30)) {
                Ok(ms) => {
                    let mut out = io::stdout();
                    if writeln!(out, "{ms}").and_then(|_| out.flush()).is_err() {
                        return;
                    }
                }
                Err(_) => stream = None,
            }
        }
        std::thread::sleep(Duration::from_secs(secs));
    }
}

fn menu_cmd(verb: &str) -> Res {
    if sys::is_root() {
        // sudo tormarchy menu add means the person behind sudo, never root.
        let user = std::env::var("SUDO_USER")
            .ok()
            .filter(|u| sys::valid_user_name(u) && u != "root");
        let Some(user) = user else {
            return Err(Fail("run this as yourself, not as root".into()));
        };
        return install::menu_as_user(&user, verb).map(|m| print(&m));
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("HOME is not set")?;
    match verb {
        "add" => print(&menu::write(&home, menu::Verb::Add)?),
        "remove" => print(&menu::write(&home, menu::Verb::Remove)?),
        "status" | "" => {
            if menu::present(&home) {
                println!("Tor is in the Omarchy menu.");
            } else {
                println!("Tor is not in the Omarchy menu. Add it with: tormarchy menu add");
            }
        }
        _ => return Err(Fail("Usage: tormarchy menu <add|remove|status>".into())),
    }
    Ok(())
}

fn help() -> String {
    "tormarchy -- route traffic through Tor, from the Omarchy bar.

  connect [--dry-run] [lan|strict|socks]   Connect. --dry-run prints the
                                           ruleset without applying it.
  disconnect                               Remove the rules and stop tor.
  mode <lan|strict|socks>                  Change mode, live if connected.
  status [--json]                          Current state.
  ip [--json]                              Circuit path and exit, from Tor's
                                           control port. Makes no request.
  newnym                                   New circuit.
  exit <cc>|auto|--list                    Pin or clear the exit country.
  exit --exclude <cc,cc>|none              Never exit in these countries.
  bridge <status|set|check|on|off|transports>
                                           Use bridges where Tor is blocked.
  browser [name]|--list                    Launch a browser, in an instance of
                                           its own, that the firewall lets
                                           reach nothing but Tor.
  speed [--json]                           One latency measurement.
  pingd [seconds]                          Stream live latency, one per line.
  doctor [--json]                          Leak checks.
  boot <status|enable|disable>             Reconnect at boot if Tor was on.
  menu <add|remove|status>                 Tor in the Omarchy menu. setup adds it.
  panic                                    Remove every rule. The escape hatch.
  setup / uninstall [--purge]              Install or remove the system side.
                                           Terminal + sudo only.
  version                                  The installed version.

Modes:
  strict   Every packet through Tor. Local network dropped too.
  lan      All internet traffic through Tor; local network still reachable.
  socks    No system-wide change; use 'tormarchy browser'.
"
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pkexec_command_is_a_subcommand() {
        for c in PKEXEC_COMMANDS {
            assert!(SUBCOMMANDS.contains(&c));
        }
    }

    #[test]
    fn setup_and_uninstall_never_run_through_polkit() {
        assert!(!PKEXEC_COMMANDS.contains(&"setup"));
        assert!(!PKEXEC_COMMANDS.contains(&"uninstall"));
        assert!(
            !PKEXEC_COMMANDS.contains(&"browser-exec"),
            "it becomes whatever program it is handed"
        );
        assert!(!PKEXEC_COMMANDS.contains(&"boot"));
        assert!(!PKEXEC_COMMANDS.contains(&"menu"));
    }

    #[test]
    fn the_help_names_every_documented_subcommand() {
        let h = help();
        for c in SUBCOMMANDS.iter().filter(|c| !c.starts_with("browser-")) {
            assert!(h.contains(c), "{c} is not in the help");
        }
    }
}
