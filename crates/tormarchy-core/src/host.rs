//! Everything that changes or asks the machine, behind one trait.
//!
//! The operations in [`crate::ops`] decide; a [`Host`] does. Production uses
//! [`System`]; tests substitute a fake that records what was asked and can
//! misbehave on cue -- a panic landing mid-apply, a tor that stops while a
//! connect waits -- which is how the ordering guarantees are tested at all.

use std::net::{SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::control::Endpoint;
use crate::paths::Paths;
use crate::sys::{self, Output, Run};
use crate::{socks, tor};

pub trait Host {
    fn paths(&self) -> &Paths;

    fn tor_running(&self) -> bool;
    /// `systemctl <args>`; true on success.
    fn systemctl(&self, args: &[&str]) -> bool;
    /// `nft <args>`, with `stdin` as its input when given.
    fn nft(&self, args: &[&str], stdin: Option<&str>) -> Output;
    /// tor's own parser over the full config; the error is its first complaint.
    fn tor_verify(&self) -> Result<(), String>;
    fn tor_installed(&self) -> bool;
    /// The program that runs a pluggable transport, if this machine has it.
    fn transport_binary(&self, transport: &str) -> Option<PathBuf>;

    /// Bootstrap percentage, or None when the control port did not answer.
    fn bootstrap(&self) -> Option<u8>;
    fn circuit_established(&self) -> bool;
    fn newnym(&self) -> Result<(), String>;
    /// One lookup through tor's DNSPort, answered NOERROR.
    fn dns_probe(&self) -> bool;
    /// One real round trip through the SOCKS port, in milliseconds.
    fn route_probe_ms(&self, url: &str) -> Option<u64>;
    /// Copies tor's fresh cookie where the invoking user can read it.
    fn publish_cookie(&self) -> bool;
    /// Starts tor.service, clearing a tripped start limit first; the error
    /// carries what tor's journal said.
    fn start_tor(&self) -> Result<(), String>;
    /// Programs holding a live connection to something off this machine.
    fn live_apps(&self) -> Vec<String>;

    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
    fn now(&self) -> Instant {
        Instant::now()
    }
    /// Lines of progress for a person watching, on stdout.
    fn say(&self, line: &str) {
        println!("{line}");
    }
    fn warn(&self, line: &str) {
        eprintln!("{line}");
    }
}

pub const SOCKS_PORT: u16 = 9050;
pub const DNS_PORT: u16 = 9053;
pub const TRANS_PORT: u16 = 9040;

/// The machine itself.
pub struct System {
    pub paths: Paths,
    /// The user a root command acts for: whose cookie copy, whose menu.
    pub invoking_user: Option<String>,
}

impl System {
    pub fn new(paths: Paths) -> Self {
        System {
            paths,
            invoking_user: invoking_user(),
        }
    }

    pub fn endpoint(&self) -> Endpoint {
        Endpoint::new(self.paths.cookie_candidates())
    }
}

/// Who ran this: pkexec's caller, else sudo's.
pub fn invoking_user() -> Option<String> {
    if let Ok(uid) = std::env::var("PKEXEC_UID") {
        if let Ok(uid) = uid.parse::<u32>() {
            return sys::user_name(uid);
        }
    }
    std::env::var("SUDO_USER").ok().filter(|u| !u.is_empty())
}

impl Host for System {
    fn paths(&self) -> &Paths {
        &self.paths
    }

    fn tor_running(&self) -> bool {
        Run::new("systemctl", ["is-active", "--quiet", "tor.service"])
            .timeout(Duration::from_secs(10))
            .run()
            .ok()
    }

    fn systemctl(&self, args: &[&str]) -> bool {
        Run::new("systemctl", args)
            .timeout(Duration::from_secs(120))
            .run()
            .ok()
    }

    fn nft(&self, args: &[&str], stdin: Option<&str>) -> Output {
        let mut run = Run::new("nft", args).timeout(Duration::from_secs(30));
        if let Some(input) = stdin {
            run = run.stdin(input.as_bytes().to_vec());
        }
        run.run()
    }

    fn tor_verify(&self) -> Result<(), String> {
        let torrc = self.paths.torrc.to_string_lossy().into_owned();
        let out = Run::new("tor", ["--verify-config", "-f", &torrc])
            .timeout(Duration::from_secs(60))
            .run();
        if out.ok() {
            return Ok(());
        }
        let log = format!("{}\n{}", out.stdout_str(), out.stderr_str());
        let pick = log
            .lines()
            .find(|l| {
                (l.contains("[err]") || l.contains("[warn]"))
                    && (l.contains("ridge") || l.contains("ransport"))
            })
            .or_else(|| {
                log.lines()
                    .find(|l| l.contains("[err]") || l.contains("[warn]"))
            })
            .unwrap_or("tor rejected the configuration");
        Err(pick.rsplit("] ").next().unwrap_or(pick).trim().to_string())
    }

    fn tor_installed(&self) -> bool {
        sys::which("tor").is_some()
    }

    fn transport_binary(&self, transport: &str) -> Option<PathBuf> {
        let candidates: &[&str] = match transport {
            "obfs4" | "obfs3" | "scramblesuit" => &["lyrebird", "obfs4proxy"],
            "meek" | "meek_lite" => &["meek-client"],
            "snowflake" => &["snowflake-client"],
            "webtunnel" => &["webtunnel-client"],
            _ => &[],
        };
        candidates.iter().find_map(|c| sys::which(c))
    }

    fn bootstrap(&self) -> Option<u8> {
        let ep = self.endpoint();
        let mut ask = tor::OneShot {
            endpoint: &ep,
            timeout: Duration::from_secs(5),
        };
        tor::vitals(&mut ask).ok().map(|v| v.bootstrap)
    }

    fn circuit_established(&self) -> bool {
        let ep = self.endpoint();
        let mut ask = tor::OneShot {
            endpoint: &ep,
            timeout: Duration::from_secs(5),
        };
        tor::circuit_established(&mut ask)
    }

    fn newnym(&self) -> Result<(), String> {
        let ep = self.endpoint();
        let mut ask = tor::OneShot {
            endpoint: &ep,
            timeout: Duration::from_secs(5),
        };
        tor::newnym(&mut ask)
    }

    fn dns_probe(&self) -> bool {
        dns_probe(DNS_PORT, ROUTE_PROBE_TIMEOUT)
    }

    fn route_probe_ms(&self, url: &str) -> Option<u64> {
        socks::time_request(SOCKS_PORT, url, ROUTE_PROBE_TIMEOUT).ok()
    }

    fn publish_cookie(&self) -> bool {
        publish_cookie(&self.paths, self.invoking_user.as_deref())
    }

    fn start_tor(&self) -> Result<(), String> {
        // A tripped rate limiter refuses with "start request repeated too
        // quickly", which describes systemd's bookkeeping, not tor -- and masks
        // the real error for as long as the window lasts.
        self.systemctl(&["reset-failed", "tor.service"]);
        if self.systemctl(&["start", "tor.service"]) {
            return Ok(());
        }
        let log = Run::new(
            "journalctl",
            ["-u", "tor.service", "-n", "25", "--no-pager"],
        )
        .timeout(Duration::from_secs(10))
        .run()
        .stdout_str();
        let detail: Vec<String> = log
            .lines()
            .filter(|l| {
                let l = l.to_ascii_lowercase();
                ["err", "warn", "denied", "couldn", "cannot", "unable"]
                    .iter()
                    .any(|k| l.contains(k))
            })
            .map(|l| l.rsplit("]: ").next().unwrap_or(l).trim().to_string())
            .collect();
        let tail = detail
            .iter()
            .rev()
            .take(2)
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
        Err(if tail.is_empty() {
            "could not start tor.service".into()
        } else {
            format!("could not start tor.service -- {tail}")
        })
    }

    /// anonsurf kills a hardcoded list of browsers here. The concern is real --
    /// a connection opened before the rules went on was made outside Tor -- but
    /// killing a user's programs is not this tool's decision. Asking the kernel
    /// covers everything and invents nothing.
    fn live_apps(&self) -> Vec<String> {
        let out = Run::new("ss", ["-tnp"])
            .timeout(Duration::from_secs(5))
            .run();
        let mut apps: Vec<String> = Vec::new();
        for line in out.stdout_str().lines().skip(1) {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 6 || is_local_peer(f[4]) {
                continue;
            }
            if let Some(name) = f[5].split('"').nth(1) {
                if !apps.iter().any(|a| a == name) {
                    apps.push(name.to_string());
                }
            }
        }
        apps.sort();
        apps
    }
}

fn is_local_peer(peer: &str) -> bool {
    let host = peer.rsplit_once(':').map(|(h, _)| h).unwrap_or(peer);
    if host.starts_with("127.")
        || host.starts_with("[::1]")
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || host.starts_with("169.254.")
    {
        return true;
    }
    if let Some(rest) = host.strip_prefix("172.") {
        if let Some(n) = rest.split('.').next().and_then(|n| n.parse::<u8>().ok()) {
            return (16..=31).contains(&n);
        }
    }
    false
}

pub const ROUTE_PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// check.torproject.org, type A, under a fixed id: answered NOERROR under the
/// same id means tor's resolver works.
pub fn dns_probe(port: u16, timeout: Duration) -> bool {
    const QUERY: &[u8] = b"\x7e\x51\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00\x05check\x0atorproject\x03org\x00\x00\x01\x00\x01";
    let Ok(sock) = UdpSocket::bind(SocketAddr::from(([127, 0, 0, 1], 0))) else {
        return false;
    };
    if sock
        .connect(SocketAddr::from(([127, 0, 0, 1], port)))
        .is_err()
        || sock.send(QUERY).is_err()
    {
        return false;
    }
    let deadline = Instant::now() + timeout;
    let mut buf = [0u8; 512];
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        let _ = sock.set_read_timeout(Some(left.max(Duration::from_millis(1))));
        match sock.recv(&mut buf) {
            Ok(n) if n >= 4 => {
                if buf[0] == 0x7e && buf[1] == 0x51 {
                    return buf[2] & 0x80 != 0 && buf[3] & 0x0f == 0;
                }
            }
            Ok(_) => {}
            Err(_) => return false,
        }
    }
    false
}

/// Root only. Copies tor's freshly written cookie where the invoking user can
/// read it. Always from tor's own file, never the published copy: tor writes a
/// new cookie every time it starts, and a stale copy authenticates against
/// nothing. tor can write it after `systemctl start` returns, so this waits a
/// little for it.
pub fn publish_cookie(paths: &Paths, user: Option<&str>) -> bool {
    if !sys::is_root() {
        return false;
    }
    let Some(user) = user.filter(|u| sys::valid_user_name(u) && *u != "root") else {
        return false;
    };
    let Some((uid, gid, _)) = sys::user_info(user) else {
        return false;
    };
    let mut cookie = None;
    for _ in 0..50 {
        cookie = paths
            .tor_cookies
            .iter()
            .find_map(|p| sys::read_bounded(p, 64).ok().filter(|c| !c.is_empty()));
        if cookie.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let Some(cookie) = cookie else { return false };
    if sys::ensure_dir(&paths.state_dir, 0o755).is_err() {
        return false;
    }
    let ok = sys::write_atomic(&paths.user_cookie, &cookie, 0o600).is_ok();
    ok && sys::lchown(&paths.user_cookie, uid, gid).is_ok()
}
