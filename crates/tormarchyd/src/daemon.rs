//! The state machine behind the panel.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tormarchy_core::control::{Conn, Endpoint, Reply};
use tormarchy_core::doctor::{self, Report, Status};
use tormarchy_core::host::{Host, System, SOCKS_PORT};
use tormarchy_core::paths::{Paths, HELPER_PATH};
use tormarchy_core::status::Facts;
use tormarchy_core::sys::{self, Run};
use tormarchy_core::view::{self, Conn as ConnState, Note, Policy};
use tormarchy_core::{bridges, exits, socks, state, tor, Mode};

pub enum Ev {
    Input(Value),
    InputClosed,
    Tick,
    Tor(u64, Reply),
    TorClosed(u64),
    Action {
        kind: Kind,
        code: Option<i32>,
        stdout: String,
        stderr: String,
    },
    Latency(u64, Result<u64, String>),
    Speed(Result<u64, String>),
    Leak(Report),
    Exits(Result<Vec<String>, String>),
    Clipboard(Result<String, String>),
    Notified {
        status: Option<i32>,
        id: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// connect, disconnect, mode, exit, bridge on/off, panic.
    Main,
    /// The disconnect that cancels a connect still waiting on tor.
    Cancel,
    /// `bridge set`, its lines on stdin.
    BridgeSet,
}

struct Settings {
    mode: Mode,
    speed_interval: u64,
    refresh_interval: u64,
    policy: Policy,
    recent_exits: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            mode: Mode::Lan,
            speed_interval: 3,
            refresh_interval: 5,
            policy: Policy::Problems,
            recent_exits: vec![],
        }
    }
}

const ACTION_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const SETTLE: Duration = Duration::from_secs(12);
const STATUS_LINGER: Duration = Duration::from_millis(2600);
/// tor is briefly not running while it restarts -- a bridge change, an exit
/// change falling back to a restart -- and that is not the kill switch holding.
/// Only a "blocked" that lasts this long is real.
const BLOCKED_GRACE: Duration = Duration::from_secs(4);

pub struct Daemon {
    tx: Sender<Ev>,
    paths: Paths,
    host: System,
    /// False under a test root: systemd's tor.service is the machine's, not
    /// the test world's, and a host running tor would answer for a fake that
    /// has stopped.
    ask_systemd: bool,
    helper: String,
    settings: Settings,
    plugin_version: Option<String>,
    watchers: u32,

    conn: Option<Conn>,
    conn_gen: u64,
    last_conn_try: Option<Instant>,
    last_systemctl: Option<Instant>,
    facts: Facts,
    path_dirty: bool,
    last_path: Option<Instant>,

    // Optimistic state, so the switch reacts the instant it is clicked rather
    // than after a bootstrap that can take ten seconds. None while following
    // the real state; Some while a toggle catches up.
    desired: Option<bool>,
    settle_until: Option<Instant>,
    action_running: bool,
    action_was_connect: bool,
    cancel_running: bool,
    cancelled: bool,
    bridge_set_running: bool,
    bridge_action_pending: bool,
    action_status: String,
    action_status_until: Option<Instant>,
    last_error: String,

    latency: u64,
    speed_error: String,
    measuring: bool,
    ping_gen: u64,
    ping_stop: Option<Arc<AtomicBool>>,
    ping_held_until: Option<Instant>,

    leak: Option<Report>,
    leak_at: u64,
    leak_error: String,
    checking: bool,

    exit_countries: Vec<String>,
    exits_loaded: bool,
    loading_exits: bool,

    /// A paste on its way in. Only ever in memory, and dropped on apply, on
    /// cancel, and whenever nobody is looking.
    pending_bridges: String,
    preview: Option<bridges::Report>,
    bridge_message: String,
    reading_clipboard: bool,

    notified: Option<ConnState>,
    blocked_since: Option<Instant>,
    toast_id: String,
    notify_running: bool,
    queued_note: Option<Note>,
    notifications_available: bool,

    last_view: String,
}

fn tor_events() -> &'static [&'static str] {
    &["STATUS_CLIENT", "CIRC"]
}

impl Daemon {
    pub fn new(tx: Sender<Ev>) -> Daemon {
        // A test seam, honoured here and nowhere else: this process runs as the
        // user, so pointing it at other files reaches nothing that user could
        // not already read. The root helper has no such variable.
        let test_root = std::env::var_os("TORMARCHY_ROOT");
        let ask_systemd = test_root.is_none();
        let paths = match test_root {
            Some(root) => Paths::under(std::path::Path::new(&root)),
            None => Paths::system(),
        };
        Daemon {
            tx,
            host: System::new(paths.clone()),
            ask_systemd,
            paths,
            helper: std::env::var("TORMARCHY_HELPER").unwrap_or_else(|_| HELPER_PATH.into()),
            settings: Settings::default(),
            plugin_version: None,
            watchers: 0,
            conn: None,
            conn_gen: 0,
            last_conn_try: None,
            last_systemctl: None,
            facts: Facts::default(),
            path_dirty: true,
            last_path: None,
            desired: None,
            settle_until: None,
            action_running: false,
            action_was_connect: false,
            cancel_running: false,
            cancelled: false,
            bridge_set_running: false,
            bridge_action_pending: false,
            action_status: String::new(),
            action_status_until: None,
            last_error: String::new(),
            latency: 0,
            speed_error: String::new(),
            measuring: false,
            ping_gen: 0,
            ping_stop: None,
            ping_held_until: None,
            leak: None,
            leak_at: 0,
            leak_error: String::new(),
            checking: false,
            exit_countries: vec![],
            exits_loaded: false,
            loading_exits: false,
            pending_bridges: String::new(),
            preview: None,
            bridge_message: String::new(),
            reading_clipboard: false,
            notified: None,
            blocked_since: None,
            toast_id: String::new(),
            notify_running: false,
            queued_note: None,
            notifications_available: true,
            last_view: String::new(),
        }
    }

    pub fn run(mut self, rx: Receiver<Ev>) {
        self.refresh(true);
        self.emit();
        while let Ok(ev) = rx.recv() {
            match ev {
                Ev::InputClosed => break,
                Ev::Input(v) => self.command(&v),
                Ev::Tick => self.tick(),
                Ev::Tor(gen, reply) if gen == self.conn_gen => self.tor_event(&reply),
                Ev::Tor(..) => {}
                Ev::TorClosed(gen) if gen == self.conn_gen => {
                    self.conn = None;
                    self.facts.control_port = false;
                    self.refresh(true);
                }
                Ev::TorClosed(_) => {}
                Ev::Action {
                    kind,
                    code,
                    stdout,
                    stderr,
                } => self.action_done(kind, code, stdout, stderr),
                Ev::Latency(gen, r) if gen == self.ping_gen => match r {
                    Ok(ms) => {
                        self.latency = ms;
                        self.speed_error.clear();
                        self.measuring = false;
                    }
                    Err(e) => self.speed_error = view::elide(&e, 120),
                },
                Ev::Latency(..) => {}
                Ev::Speed(r) => {
                    self.measuring = false;
                    match r {
                        Ok(ms) => {
                            self.latency = ms;
                            self.speed_error.clear();
                        }
                        Err(e) => {
                            self.latency = 0;
                            self.speed_error = e;
                        }
                    }
                }
                Ev::Leak(r) => {
                    self.checking = false;
                    self.leak_at = sys::now_unix();
                    self.leak = Some(r);
                }
                Ev::Exits(r) => {
                    self.loading_exits = false;
                    if let Ok(list) = r {
                        self.exit_countries = list;
                        self.exits_loaded = true;
                    }
                }
                Ev::Clipboard(r) => self.clipboard_done(r),
                Ev::Notified { status, id } => self.notified_done(status, id),
            }
            self.sync_ping();
            self.emit();
        }
        if let Some(stop) = self.ping_stop.take() {
            stop.store(true, Ordering::SeqCst);
        }
    }

    // ---- derived state, as Service.qml had it ----------------------------

    fn effective_mode(&self) -> Mode {
        self.facts.mode_or_default().unwrap_or(self.settings.mode)
    }

    /// What "on" means depends on the mode: rules present, or for browser-only
    /// -- which installs none by design -- tor itself being up.
    fn live_connection(&self) -> bool {
        if self.effective_mode() == Mode::Socks {
            self.facts.tor_running
        } else {
            self.facts.connected
        }
    }

    fn active(&self) -> bool {
        self.desired.unwrap_or_else(|| self.live_connection())
    }

    /// Only from a figure tor gave: with the control port unreachable the
    /// bootstrap reads 0 whatever tor is doing.
    fn bootstrapping(&self) -> bool {
        self.facts.tor_running
            && !self.facts.connected
            && self.facts.control_port
            && self.facts.bootstrap < 100
    }

    fn busy(&self) -> bool {
        self.action_running || self.cancel_running || self.bridge_set_running
    }

    /// A connect spends most of its life waiting for tor, and turning it off
    /// then cancels it.
    fn can_toggle(&self) -> bool {
        !self.busy() || (self.action_running && self.desired == Some(true) && !self.cancel_running)
    }

    /// A circuit worth measuring: the switch says on and the connection is
    /// really up rather than merely asked for.
    fn circuit_ready(&self) -> bool {
        self.facts.tor_running
            && self.active()
            && !self.bootstrapping()
            && (self.effective_mode() == Mode::Socks || self.facts.connected)
    }

    fn can_new_circuit(&self) -> bool {
        self.facts.tor_running && self.active() && !self.bootstrapping()
    }

    fn watching(&self) -> bool {
        self.watchers > 0
    }

    // ---- reading the machine ---------------------------------------------

    fn endpoint(&self) -> Endpoint {
        let mut ep = Endpoint::new(self.paths.cookie_candidates());
        if let Some(port) = std::env::var("TORMARCHY_CONTROL_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
        {
            ep.port = port;
        }
        ep
    }

    fn ensure_conn(&mut self) {
        if self.conn.is_some() {
            return;
        }
        if self
            .last_conn_try
            .map(|t| t.elapsed() < Duration::from_secs(1))
            .unwrap_or(false)
        {
            return;
        }
        self.last_conn_try = Some(Instant::now());
        let ep = self.endpoint();
        let Ok((mut conn, events)) = Conn::open(&ep, tor_events()) else {
            return;
        };
        self.conn_gen += 1;
        let gen = self.conn_gen;
        let tx = self.tx.clone();
        thread::spawn(move || {
            for ev in events {
                if tx.send(Ev::Tor(gen, ev)).is_err() {
                    return;
                }
            }
            let _ = tx.send(Ev::TorClosed(gen));
        });
        if let Ok(v) = tor::vitals(&mut conn) {
            self.facts.bootstrap = v.bootstrap;
            self.facts.bytes_read = v.bytes_read;
            self.facts.bytes_written = v.bytes_written;
            self.facts.control_port = true;
            self.facts.tor_running = true;
        }
        self.path_dirty = true;
        self.conn = Some(conn);
    }

    /// Everything but what tor pushes. `force` asks systemd even while the
    /// control session says tor is up.
    fn refresh(&mut self, force: bool) {
        let p = &self.paths;
        self.facts.installed = p.torrc_base.exists() && self.host.tor_installed();
        self.facts.connected = p.active_file.exists();
        self.facts.mode = state::current_mode(p);
        self.facts.requested_exit = exits::requested(p);
        self.facts.excluded_exits = exits::excluded(p);
        let next_bridges = bridges::state(p).to_string();
        if self.notified.is_some() && next_bridges != self.facts.bridges {
            let user_action =
                self.bridge_action_pending || !(self.action_was_connect && self.action_running);
            if let Some(n) = view::bridges_switched_on(
                &self.facts.bridges,
                &next_bridges,
                self.settings.policy,
                user_action,
            ) {
                self.notify(n);
            }
        }
        self.facts.bridges = next_bridges;

        self.ensure_conn();
        if self.conn.is_some() {
            self.facts.tor_running = true;
        } else {
            self.facts.control_port = false;
            let interval = if self.desired.is_some() || self.settle_until.is_some() {
                Duration::from_secs(1)
            } else {
                Duration::from_secs(self.settings.refresh_interval)
            };
            let due = self
                .last_systemctl
                .map(|t| t.elapsed() >= interval)
                .unwrap_or(true);
            if force || due {
                self.last_systemctl = Some(Instant::now());
                self.facts.tor_running = self.ask_systemd && self.host.tor_running();
            }
            if !self.facts.tor_running {
                self.facts.bootstrap = 0;
                self.facts.hops.clear();
                self.facts.exit_ip.clear();
            }
        }
        self.update_path();
        self.after_facts();
    }

    fn update_path(&mut self) {
        let watching = self.watching();
        let Some(conn) = self.conn.as_mut() else {
            return;
        };
        if watching {
            if let Ok(v) = tor::vitals(conn) {
                self.facts.bytes_read = v.bytes_read;
                self.facts.bytes_written = v.bytes_written;
                self.facts.bootstrap = v.bootstrap;
            }
        }
        if self.facts.bootstrap < 100 {
            return;
        }
        let due = self
            .last_path
            .map(|t| t.elapsed() >= Duration::from_secs(2))
            .unwrap_or(true);
        if self.path_dirty && due {
            self.last_path = Some(Instant::now());
            match tor::current_path(conn) {
                Ok(Some(p)) => {
                    self.path_dirty = false;
                    self.facts.circuit_age = p.age_seconds(sys::now_unix());
                    self.facts.exit_ip = p.exit_ip().to_string();
                    self.facts.hops = p.hops;
                }
                Ok(None) => self.facts.hops.clear(),
                Err(_) => {}
            }
        }
    }

    fn tor_event(&mut self, ev: &Reply) {
        let text = ev.first_text();
        if text.starts_with("STATUS_CLIENT") && text.contains("BOOTSTRAP") {
            self.facts.bootstrap = tor::parse_bootstrap(text);
            self.facts.control_port = true;
            if self.facts.bootstrap >= 100 {
                self.path_dirty = true;
            }
        } else if text.starts_with("STATUS_CLIENT") && text.contains("CIRCUIT_ESTABLISHED") {
            self.path_dirty = true;
        } else if let Some(rest) = text.strip_prefix("CIRC ") {
            let mut f = rest.split_whitespace();
            let _id = f.next();
            if matches!(f.next(), Some("BUILT") | Some("CLOSED") | Some("FAILED"))
                && rest.contains("PURPOSE=GENERAL")
            {
                self.path_dirty = true;
            }
        }
        self.update_path();
        self.after_facts();
    }

    /// Once a reading is applied in full: the optimistic state and the
    /// notification for any change. Never from a half-applied reading, which
    /// between "tor stopped" and "rules gone" reads as the kill switch holding.
    fn after_facts(&mut self) {
        if let Some(d) = self.desired {
            if self.live_connection() == d {
                self.desired = None;
            }
        }
        if !self.circuit_ready() {
            self.latency = 0;
            self.speed_error.clear();
            self.measuring = false;
        }
        let next = view::connection_state(
            self.facts.installed,
            self.facts.connected,
            self.facts.tor_running,
            Some(self.effective_mode()),
            self.bootstrapping(),
        );
        let Some(prev) = self.notified else {
            // The first reading is where things stand, not a change.
            self.notified = Some(next);
            return;
        };
        if next == prev {
            self.blocked_since = None;
            return;
        }
        if next == ConnState::Blocked {
            match self.blocked_since {
                None => {
                    self.blocked_since = Some(Instant::now());
                    return;
                }
                Some(t) if t.elapsed() < BLOCKED_GRACE => return,
                _ => {}
            }
        }
        self.blocked_since = None;
        let country = view::country_name(self.facts.exit_country());
        let note = view::notification_for(prev, next, self.settings.policy, &country);
        self.notified = Some(next);
        if let Some(n) = note {
            self.notify(n);
        }
    }

    fn tick(&mut self) {
        if let Some(t) = self.action_status_until {
            if Instant::now() >= t {
                self.action_status.clear();
                self.action_status_until = None;
            }
        }
        if let Some(t) = self.settle_until {
            if Instant::now() >= t {
                self.settle_until = None;
                self.desired = None;
                self.bridge_action_pending = false;
            }
        }
        if let Some(t) = self.ping_held_until {
            if Instant::now() >= t {
                self.ping_held_until = None;
            }
        }
        self.refresh(false);
    }

    // ---- commands ---------------------------------------------------------

    fn command(&mut self, v: &Value) {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        match s("cmd").as_str() {
            "hello" | "settings" => {
                if let Some(set) = v.get("settings") {
                    self.apply_settings(set);
                }
                if let Some(dir) = v.get("pluginDir").and_then(Value::as_str) {
                    self.plugin_version = plugin_version(&PathBuf::from(dir));
                }
            }
            // How many panels are open right now, as a number rather than one
            // more or one fewer: a daemon started again while two panels are
            // open is told two, not one, so closing either does not stop what
            // the other still shows.
            "watchers" => {
                let count = v.get("count").and_then(Value::as_u64).unwrap_or(0).min(64) as u32;
                let was = self.watching();
                self.watchers = count;
                if self.watching() && !was {
                    self.refresh(true);
                    if !self.exits_loaded {
                        self.load_exits();
                    }
                } else if !self.watching() && was {
                    self.cancel_bridges();
                }
            }
            "refresh" => self.refresh(true),
            "toggle" => self.toggle(),
            "connect" => self.connect(self.effective_mode()),
            "disconnect" => self.disconnect(),
            "setMode" => {
                if let Some(m) = Mode::parse(&s("mode")) {
                    if Some(m) != self.facts.mode {
                        self.act(
                            &["mode", m.as_str()],
                            None,
                            &format!("Switching to {}…", m.label()),
                        );
                    }
                }
            }
            "newCircuit" => self.new_circuit(),
            "setExit" => {
                let cc = s("cc");
                if cc == "auto" {
                    self.act(&["exit", "auto"], None, "Clearing exit country…");
                } else if let Some(code) = exits::valid_code(&cc) {
                    self.act(
                        &["exit", &code],
                        None,
                        &format!("Switching exit to {}…", code.to_uppercase()),
                    );
                }
            }
            "loadExits" => self.load_exits(),
            "measure" => self.measure(),
            "checkLeaks" => self.check_leaks(),
            "pasteBridges" => self.paste_bridges(),
            "applyBridges" => self.apply_bridges(),
            "cancelBridges" => self.cancel_bridges(),
            "setBridges" => {
                let on = v.get("on").and_then(Value::as_bool).unwrap_or(false);
                self.act(
                    &["bridge", if on { "on" } else { "off" }],
                    None,
                    if on {
                        "Turning bridges on…"
                    } else {
                        "Turning bridges off…"
                    },
                );
                self.bridge_action_pending = true;
            }
            "panic" => self.act(&["panic"], Some(false), "Removing all Tor rules…"),
            _ => {}
        }
    }

    fn apply_settings(&mut self, set: &Value) {
        let int = |k: &str, d: u64, lo: u64, hi: u64| {
            set.get(k)
                .and_then(|v| {
                    v.as_u64()
                        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                })
                .unwrap_or(d)
                .clamp(lo, hi)
        };
        self.settings.mode = set
            .get("mode")
            .and_then(Value::as_str)
            .and_then(Mode::parse)
            .unwrap_or(Mode::Lan);
        self.settings.refresh_interval = int("refreshIntervalSec", 5, 2, 300);
        self.settings.speed_interval = int("speedIntervalSec", 3, 1, 60);
        self.settings.policy = Policy::parse(
            set.get("notifications")
                .and_then(Value::as_str)
                .unwrap_or("problems"),
        );
        self.settings.recent_exits = set
            .get("recentExits")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .filter_map(exits::valid_code)
                    .take(8)
                    .collect()
            })
            .unwrap_or_default();
    }

    fn toggle(&mut self) {
        if !self.facts.installed || !self.can_toggle() {
            return;
        }
        if self.active() {
            self.disconnect();
        } else {
            self.connect(self.effective_mode());
        }
    }

    fn connect(&mut self, mode: Mode) {
        self.act(&["connect", mode.as_str()], Some(true), "Connecting…");
    }

    fn disconnect(&mut self) {
        if self.action_running && self.desired == Some(true) && !self.cancel_running {
            // The connect runs as root and cannot be killed from here, so it is
            // cancelled the way it watches for: tor stopping.
            self.desired = Some(false);
            self.set_status("Cancelling…", false);
            self.cancel_running = true;
            self.spawn_helper(Kind::Cancel, vec!["disconnect".into()], None);
            return;
        }
        self.act(&["disconnect"], Some(false), "Disconnecting…");
    }

    fn set_status(&mut self, text: &str, linger: bool) {
        self.action_status = text.to_string();
        self.action_status_until = linger.then(|| Instant::now() + STATUS_LINGER);
    }

    fn act(&mut self, args: &[&str], desired: Option<bool>, progress: &str) {
        if self.action_running {
            return;
        }
        if desired.is_some() {
            self.desired = desired;
        }
        self.action_was_connect = args.first() == Some(&"connect");
        self.bridge_action_pending = args.first() == Some(&"bridge");
        self.set_status(progress, false);
        self.action_running = true;
        self.spawn_helper(
            Kind::Main,
            args.iter().map(|s| s.to_string()).collect(),
            None,
        );
    }

    fn spawn_helper(&self, kind: Kind, args: Vec<String>, stdin: Option<String>) {
        let tx = self.tx.clone();
        let helper = self.helper.clone();
        thread::spawn(move || {
            let mut run = Run::new(&helper, args)
                .timeout(ACTION_TIMEOUT)
                .limit(64 * 1024);
            if let Some(input) = stdin {
                run = run.stdin(input.into_bytes());
            }
            let out = run.run();
            let _ = tx.send(Ev::Action {
                kind,
                code: out.status,
                stdout: out.stdout_str(),
                stderr: out.stderr_str(),
            });
        });
    }

    fn action_done(&mut self, kind: Kind, code: Option<i32>, stdout: String, stderr: String) {
        let message = |fallback: &str| {
            let t = if !stderr.trim().is_empty() {
                stderr.clone()
            } else if !stdout.trim().is_empty() {
                stdout.clone()
            } else {
                fallback.to_string()
            };
            view::elide(&t, 140)
        };
        match kind {
            Kind::Cancel => {
                self.cancel_running = false;
                // Tells the connect's own exit, which can land after this, to
                // stay quiet -- only when the cancel worked.
                self.cancelled = self.action_running && code == Some(0);
                if code != Some(0) {
                    self.desired = None;
                    self.last_error = message("Could not cancel the connection");
                    self.set_status(&self.last_error.clone(), true);
                } else {
                    self.last_error.clear();
                    self.set_status("", true);
                }
            }
            Kind::Main => {
                self.action_running = false;
                if self.cancel_running || self.cancelled {
                    // A connect that a cancel stopped fails by design.
                    self.cancelled = false;
                } else if code != Some(0) {
                    self.desired = None;
                    self.last_error = message("Tor command failed");
                    // A connect can run for minutes after the panel is closed
                    // and forgotten; its failure is worth a toast. Not the
                    // password prompt being dismissed (126), nor no helper.
                    if self.action_was_connect && !matches!(code, Some(126) | Some(127)) {
                        if let Some(n) =
                            view::connect_failed(&self.last_error, self.settings.policy)
                        {
                            self.notify(n);
                        }
                    }
                    let e = self.last_error.clone();
                    self.set_status(&e, true);
                } else {
                    self.last_error.clear();
                    self.set_status("", true);
                }
            }
            Kind::BridgeSet => {
                self.bridge_set_running = false;
                self.preview = None;
                self.bridge_message = match code {
                    Some(0) | Some(126) => String::new(),
                    _ => message("Could not switch to bridges"),
                };
                self.set_status("", true);
            }
        }
        // Everything read before is stale, and a leak report describes a
        // machine that no longer exists.
        self.forget_leak();
        self.settle_until = Some(Instant::now() + SETTLE);
        self.refresh(true);
    }

    fn forget_leak(&mut self) {
        self.leak = None;
        self.leak_at = 0;
        self.leak_error.clear();
    }

    /// NEWNYM over the control session: unprivileged, no restart, no prompt.
    fn new_circuit(&mut self) {
        if !self.can_new_circuit() {
            return;
        }
        // The reading belonged to the old path; and the stream's kept-alive
        // connection stays on the circuit it was opened on, so it is held off
        // until NEWNYM has taken effect and reopens on the new one.
        self.latency = 0;
        self.speed_error.clear();
        self.ping_held_until = Some(Instant::now() + Duration::from_millis(1500));
        let result = match self.conn.as_mut() {
            Some(c) => tor::newnym(c),
            None => Err("could not reach tor's control port".into()),
        };
        match result {
            Ok(()) => {
                self.set_status("Requesting a new circuit…", true);
                self.path_dirty = true;
            }
            Err(e) => {
                self.last_error = view::elide(&e, 140);
                self.set_status(&self.last_error.clone(), true);
            }
        }
    }

    fn load_exits(&mut self) {
        if self.loading_exits || !self.facts.tor_running {
            return;
        }
        self.loading_exits = true;
        let tx = self.tx.clone();
        let ep = self.endpoint();
        thread::spawn(move || {
            let _ = tx.send(Ev::Exits(tor::exit_countries(&ep)));
        });
    }

    fn measure(&mut self) {
        if self.measuring || !self.circuit_ready() {
            return;
        }
        self.measuring = true;
        self.speed_error.clear();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let mut last = String::from("no response through the Tor SOCKS proxy");
            for url in socks::PROBE_URLS {
                match socks::time_request(SOCKS_PORT, url, Duration::from_secs(20)) {
                    Ok(ms) => {
                        let _ = tx.send(Ev::Speed(Ok(ms)));
                        return;
                    }
                    Err(e) => last = e.to_string(),
                }
            }
            let _ = tx.send(Ev::Speed(Err(view::brief(&last, 40))));
        });
    }

    /// Only when asked: each run makes real requests, and traffic nobody asked
    /// for is a pattern this tool should not add on anyone's behalf.
    fn check_leaks(&mut self) {
        if self.checking || !self.facts.installed {
            return;
        }
        self.checking = true;
        self.leak_error.clear();
        let tx = self.tx.clone();
        let paths = self.paths.clone();
        thread::spawn(move || {
            let report = doctor::run(&System::new(paths));
            let _ = tx.send(Ev::Leak(report));
        });
    }

    /// The live latency stream: one connection held open and re-timed, and only
    /// while the panel is on screen. A request to a fixed host at a fixed
    /// interval, forever, is a beacon with a recognisable shape.
    fn sync_ping(&mut self) {
        let want = self.watching() && self.circuit_ready() && self.ping_held_until.is_none();
        match (want, self.ping_stop.is_some()) {
            (true, false) => {
                self.ping_gen += 1;
                let gen = self.ping_gen;
                let stop = Arc::new(AtomicBool::new(false));
                self.ping_stop = Some(stop.clone());
                if self.latency == 0 {
                    self.measuring = true;
                }
                let tx = self.tx.clone();
                let interval = Duration::from_secs(self.settings.speed_interval);
                thread::spawn(move || ping_loop(tx, gen, stop, interval));
            }
            (false, true) => {
                if let Some(stop) = self.ping_stop.take() {
                    stop.store(true, Ordering::SeqCst);
                }
                self.ping_gen += 1;
                self.measuring = false;
            }
            _ => {}
        }
    }

    // ---- bridges ----------------------------------------------------------

    fn cancel_bridges(&mut self) {
        self.pending_bridges.clear();
        self.preview = None;
        self.bridge_message.clear();
    }

    /// The clipboard, read with a cap and a deadline before any of it is kept:
    /// it is whatever the last app chose to send, for as long as it chooses.
    fn paste_bridges(&mut self) {
        if self.reading_clipboard || self.bridge_set_running || !self.facts.installed {
            return;
        }
        self.cancel_bridges();
        self.reading_clipboard = true;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let out = Run::new("wl-paste", ["--no-newline", "--type", "text/plain"])
                .user_path()
                .timeout(Duration::from_secs(5))
                .limit(bridges::INPUT_LIMIT)
                .run();
            let r = if out.overflowed {
                Err("Too much on the clipboard to be bridge lines.".to_string())
            } else if out.timed_out || out.status.map(|s| s > 1).unwrap_or(true) {
                // 1 is nothing copied, or nothing copied as text: an empty paste.
                Err("Could not read the clipboard.".to_string())
            } else {
                Ok(String::from_utf8_lossy(&out.stdout).into_owned())
            };
            let _ = tx.send(Ev::Clipboard(r));
        });
    }

    fn clipboard_done(&mut self, r: Result<String, String>) {
        self.reading_clipboard = false;
        let text = match r {
            Ok(t) => t,
            Err(msg) => {
                self.bridge_message = msg;
                return;
            }
        };
        if text.trim().is_empty() {
            self.bridge_message = "The clipboard is empty. Copy your bridge lines first.".into();
            return;
        }
        // Checked here, in this process: the lines never go anywhere to be
        // previewed.
        let Ok(lines) = bridges::normalize(text.as_bytes()) else {
            self.bridge_message = "Too much on the clipboard to be bridge lines.".into();
            return;
        };
        let (counts, invalid) = bridges::count(&lines);
        if counts.is_empty() {
            self.bridge_message =
                "No bridge lines on the clipboard. Each starts with a transport, like obfs4."
                    .into();
            return;
        }
        let mut report = bridges::report(&self.host, &counts);
        report.invalid_lines = Some(invalid);
        self.preview = Some(report);
        self.pending_bridges = lines;
    }

    fn apply_bridges(&mut self) {
        let Some(preview) = &self.preview else { return };
        if self.busy()
            || self.pending_bridges.is_empty()
            || preview.invalid_lines.unwrap_or(0) > 0
            || preview.transports.iter().any(|t| !t.available)
        {
            return;
        }
        self.bridge_action_pending = true;
        self.bridge_set_running = true;
        self.set_status("Switching to bridges…", false);
        // Over stdin, never argv where every process on the machine can read
        // it, and never a file; dropped from here the moment it is handed over.
        let lines = std::mem::take(&mut self.pending_bridges);
        self.spawn_helper(
            Kind::BridgeSet,
            vec!["bridge".into(), "set".into()],
            Some(lines),
        );
    }

    // ---- notifications ----------------------------------------------------

    /// One at a time, newest wins: each replaces the previous toast, so a burst
    /// of changes ends as one toast saying where things stand.
    fn notify(&mut self, n: Note) {
        if !self.notifications_available {
            return;
        }
        if self.notify_running {
            self.queued_note = Some(n);
            return;
        }
        self.notify_running = true;
        let argv = view::notification_argv(&n, Some(&self.toast_id));
        let tx = self.tx.clone();
        thread::spawn(move || {
            let out = Run::new(&argv[0], &argv[1..])
                .user_path()
                .timeout(Duration::from_secs(10))
                .limit(64)
                .run();
            let _ = tx.send(Ev::Notified {
                status: out.status,
                id: out.stdout_str().trim().to_string(),
            });
        });
    }

    fn notified_done(&mut self, status: Option<i32>, id: String) {
        self.notify_running = false;
        // An Omarchy without the sender: say nothing rather than fall back to
        // notify-send, and stop asking.
        if status == Some(127) {
            self.notifications_available = false;
            self.queued_note = None;
            return;
        }
        if status == Some(0) && !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) {
            self.toast_id = id;
        }
        if let Some(n) = self.queued_note.take() {
            self.notify(n);
        }
    }

    // ---- the view ---------------------------------------------------------

    fn view(&self) -> Value {
        let f = &self.facts;
        let mode = self.effective_mode();
        let active = self.active();
        let bootstrapping = self.bootstrapping();
        let blocked = f.connected && !f.tor_running;
        let routed = active && f.connected && f.tor_running;
        let browser = active && mode == Mode::Socks && f.tor_running && !bootstrapping;
        let bar = if blocked {
            "blocked"
        } else if routed {
            "routed"
        } else if browser {
            "browser"
        } else if active {
            "connecting"
        } else {
            "off"
        };
        let settling = bootstrapping || (bar == "connecting" && f.tor_running);
        let ready = self.circuit_ready();
        let status_text = f.status_text();

        let warnings = f.warnings();
        let warning = warnings.first().cloned().unwrap_or_default();

        let hops: Vec<Value> = f
            .hops
            .iter()
            .enumerate()
            .map(|(i, h)| json!({"label": view::hop_label(&h.cc), "tip": view::hop_tooltip(h, i, f.hops.len()), "exit": i + 1 == f.hops.len()}))
            .collect();

        let leak = match &self.leak {
            Some(r) => {
                let overall = view::doctor_overall(&r.checks);
                let ago = view::format_ago(sys::now_unix().saturating_sub(self.leak_at));
                json!({
                    "glyph": if self.checking { "󰔟" } else { view::doctor_glyph(Some(overall)) },
                    "failed": overall == Status::Fail,
                    "subtitle": if self.checking { "Checking…".to_string() } else { format!("{} · {ago}", view::doctor_summary(&r.checks)) },
                    "subtitleError": false,
                    "checking": self.checking,
                    "hasResult": true,
                    "checks": r.checks.iter().map(|c| json!({
                        "glyph": view::doctor_glyph(Some(c.status)),
                        "label": view::check_label(c),
                        "tip": view::check_tip(c),
                        "status": c.status,
                    })).collect::<Vec<_>>(),
                })
            }
            None => json!({
                "glyph": if self.checking { "󰔟" } else { "󰕥" },
                "failed": false,
                "subtitle": if self.checking { "Checking…".to_string() } else if !self.leak_error.is_empty() { view::brief(&self.leak_error, 24) } else { "Not run yet".to_string() },
                "subtitleError": !self.leak_error.is_empty() && !self.checking,
                "checking": self.checking,
                "hasResult": false,
                "checks": [],
            }),
        };

        let saved = bridges::saved_counts(&self.paths);
        let saved_total: usize = saved.values().sum();
        let saved_names: Vec<String> = saved.keys().cloned().collect();
        let shown: Option<bridges::Report> = self
            .preview
            .clone()
            .or_else(|| (!saved.is_empty()).then(|| bridges::report(&self.host, &saved)));
        let chips: Vec<Value> = shown
            .as_ref()
            .map(|r| {
                r.transports
                    .iter()
                    .map(|t| json!({
                        "label": format!("{} {}", t.count, t.name),
                        "available": t.available,
                        "tip": if t.available { String::new() } else { view::tip(&format!("{} is not installed. Install it with: {}", t.name, t.install_hint)) },
                    }))
                    .collect()
            })
            .unwrap_or_default();
        let invalid = self
            .preview
            .as_ref()
            .and_then(|p| p.invalid_lines)
            .unwrap_or(0);
        let can_apply = self
            .preview
            .as_ref()
            .map(|p| p.transports.iter().all(|t| t.available))
            .unwrap_or(false)
            && invalid == 0
            && !self.busy()
            && !self.reading_clipboard;

        json!({
            "protocol": 1,
            "version": tormarchy_core::VERSION,
            "installed": f.installed,
            "helperStale": self.plugin_version.as_deref().map(|v| v != tormarchy_core::VERSION).unwrap_or(false),
            "statusText": status_text,
            "bar": bar,
            "active": active,
            "busy": self.busy(),
            "canToggle": self.can_toggle(),
            "switchCaption": if settling { format!("{}%", f.bootstrap) } else if active { "ON".into() } else { "OFF".into() },
            "switchCaptionLit": settling,
            "mode": mode.as_str(),
            "modes": Mode::DISPLAY_ORDER.iter().map(|m| json!({"value": m.as_str(), "label": m.label(), "tip": view::tip(m.tooltip()), "selected": *m == mode})).collect::<Vec<_>>(),
            "exitCaption": if ready && !f.exit_country().is_empty() { format!("EXIT · {}", f.exit_country().to_uppercase()) } else { "EXIT".into() },
            "exitValue": f.requested_exit.clone().unwrap_or_else(|| "auto".into()),
            "exitOptions": view::exit_options(&self.exit_countries, &self.settings.recent_exits, &f.excluded_exits, f.requested_exit.as_deref()),
            "warning": {"short": if warning.is_empty() { String::new() } else { view::brief_warning(&warning) }, "full": warning},
            "latency": {
                "text": view::format_latency(self.latency),
                "fraction": view::latency_fraction(self.latency),
                "measuring": self.measuring,
                "caption": if !self.speed_error.is_empty() { view::brief(&self.speed_error, 22) } else if self.measuring { "Measuring…".to_string() } else { view::latency_verdict(self.latency).to_string() },
                "captionError": !self.speed_error.is_empty(),
            },
            "canNewCircuit": self.can_new_circuit(),
            "stats": {
                "visible": ready && !f.hops.is_empty(),
                "hops": hops,
                "traffic": format!("\u{2193} {} \u{2191} {}", view::format_bytes(f.bytes_read), view::format_bytes(f.bytes_written)),
                "age": view::format_age(f.circuit_age),
            },
            "leak": leak,
            "bridges": {
                "state": f.bridges,
                "subtitle": view::bridge_state_label(&f.bridges, saved_total, &saved_names),
                "hasPreview": self.preview.is_some(),
                "previewText": self.preview.as_ref().map(|p| format!("Pasted · {}", view::bridge_count_text(p.count))).unwrap_or_default(),
                "chips": chips,
                "invalidLabel": match invalid { 0 => String::new(), 1 => "1 invalid line".into(), n => format!("{n} invalid lines") },
                "message": {"short": view::brief(&self.bridge_message, view::BRIEF_LIMIT), "full": self.bridge_message},
                "canApply": can_apply,
                "working": self.reading_clipboard || self.bridge_set_running,
                "toggleLabel": match f.bridges.as_str() { "active" => "Turn off", "stashed" => "Turn on", _ => "" },
            },
            "error": {"short": view::brief(&self.last_error, view::BRIEF_LIMIT), "full": self.last_error},
            "actionStatus": self.action_status,
        })
    }

    fn emit(&mut self) {
        let line = json!({"view": self.view()}).to_string();
        if line == self.last_view {
            return;
        }
        let mut out = std::io::stdout().lock();
        if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
            // Nobody is reading: the shell went away.
            std::process::exit(0);
        }
        self.last_view = line;
    }
}

fn ping_loop(tx: Sender<Ev>, gen: u64, stop: Arc<AtomicBool>, interval: Duration) {
    let mut stream: Option<socks::Stream> = None;
    while !stop.load(Ordering::SeqCst) {
        if stream.is_none() {
            match socks::Stream::open(SOCKS_PORT, socks::LATENCY_URL, Duration::from_secs(30)) {
                Ok(s) => stream = Some(s),
                Err(e) => {
                    if tx.send(Ev::Latency(gen, Err(e.to_string()))).is_err() {
                        return;
                    }
                }
            }
        }
        if let Some(s) = stream.as_mut() {
            match s.time_head(Duration::from_secs(30)) {
                Ok(ms) => {
                    if tx.send(Ev::Latency(gen, Ok(ms))).is_err() {
                        return;
                    }
                }
                Err(_) => stream = None,
            }
        }
        // Sleeps in small steps so closing the panel stops it at once.
        let until = Instant::now() + interval;
        while Instant::now() < until {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}

/// The version of the plugin the panel was loaded from. A plugin update does
/// not touch the installed helper; only setup does, so a difference is the
/// panel's cue to offer it.
fn plugin_version(dir: &std::path::Path) -> Option<String> {
    let text = sys::read_bounded_string(&dir.join("manifest.json"), 64 * 1024)?;
    let v: Value = serde_json::from_str(&text).ok()?;
    v.get("version").and_then(Value::as_str).map(str::to_string)
}

/// The panel's QML is tested against these views, in tests/qml/fixtures. They
/// are this daemon's own output for each state the panel can be in, so the QML
/// can never be tested against a view the daemon would not send. Regenerate with
/// TORMARCHY_UPDATE_FIXTURES=1 cargo test.
#[cfg(test)]
mod golden {
    use super::*;
    use tormarchy_core::doctor::Check;
    use tormarchy_core::tor::Hop;

    fn daemon(root: &std::path::Path) -> Daemon {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut d = Daemon::new(tx);
        d.paths = Paths::under(root);
        d.host = System::new(d.paths.clone());
        d.notified = Some(ConnState::Off);
        d.facts.installed = true;
        d.facts.bridges = "off".into();
        d.exit_countries = ["de", "nl", "se", "ch", "us", "fr"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        d.settings.recent_exits = vec!["se".into()];
        d
    }

    fn routed(d: &mut Daemon) {
        d.facts.tor_running = true;
        d.facts.control_port = true;
        d.facts.connected = true;
        d.facts.bootstrap = 100;
        d.facts.mode = Some(Mode::Lan);
        d.facts.hops = vec![
            Hop {
                cc: "se".into(),
                nickname: "frostbite".into(),
                flags: vec!["Guard".into(), "Fast".into(), "Stable".into()],
                bandwidth_kbps: 21400,
                ip: "192.0.2.1".into(),
            },
            Hop {
                cc: "ch".into(),
                nickname: "alpenglow".into(),
                flags: vec!["Fast".into()],
                bandwidth_kbps: 8300,
                ip: "192.0.2.2".into(),
            },
            Hop {
                cc: "de".into(),
                nickname: "rheinexit".into(),
                flags: vec!["Exit".into(), "Fast".into()],
                bandwidth_kbps: 640,
                ip: "192.0.2.3".into(),
            },
        ];
        d.facts.circuit_age = 135;
        d.facts.bytes_read = 4_400_000;
        d.facts.bytes_written = 312_000;
        d.latency = 612;
    }

    fn check(id: &str, status: Status, text: &str, detail: &str) -> Check {
        Check {
            id: id.into(),
            group: String::new(),
            status,
            text: text.into(),
            detail: detail.into(),
        }
    }

    fn report(fail: bool) -> Report {
        let mut checks = vec![
            check("tor", Status::Pass, "tor.service is running", ""),
            check("bootstrap", Status::Pass, "bootstrapped (100%)", ""),
            check(
                "control",
                Status::Pass,
                "control cookie readable at /run/tormarchy/control.cookie",
                "",
            ),
            check(
                "exit",
                Status::Pass,
                "check.torproject.org confirms this connection is Tor",
                "exit as seen from outside: 192.0.2.3",
            ),
            check("ipv6", Status::Pass, "IPv6 is blocked", ""),
            check(
                "ipv6-stack",
                Status::Warn,
                "IPv6 is filtered but the kernel stack is still up",
                "",
            ),
            check(
                "dns",
                Status::Pass,
                "DNS resolves (through Tor's DNSPort)",
                "",
            ),
            check(
                "forwarding",
                Status::Pass,
                "IP forwarding is off, so nothing routes through this machine",
                "",
            ),
            check("clock", Status::Pass, "clock is NTP-synchronised", ""),
        ];
        if fail {
            checks[3] = check(
                "exit",
                Status::Fail,
                "check.torproject.org says this is NOT Tor -- traffic is escaping the redirect",
                "",
            );
        }
        let failures = checks.iter().filter(|c| c.status == Status::Fail).count();
        Report {
            ok: failures == 0,
            failures,
            checked_at: String::new(),
            mode: "lan".into(),
            checks,
            connected: true,
            tor_running: true,
        }
    }

    fn states(root: &std::path::Path) -> Vec<(&'static str, Value)> {
        let mut out = Vec::new();
        let mut d = daemon(root);
        d.facts.installed = false;
        out.push(("view-not-installed", d.view()));

        let d = daemon(root);
        out.push(("view-off", d.view()));

        let mut d = daemon(root);
        d.facts.tor_running = true;
        d.facts.control_port = true;
        d.facts.bootstrap = 45;
        d.desired = Some(true);
        d.action_running = true;
        out.push(("view-boot", d.view()));

        let mut d = daemon(root);
        routed(&mut d);
        out.push(("view-on", d.view()));

        let mut d = daemon(root);
        routed(&mut d);
        d.facts.tor_running = false;
        d.facts.control_port = false;
        d.facts.hops.clear();
        d.latency = 0;
        out.push(("view-blocked", d.view()));

        let mut d = daemon(root);
        routed(&mut d);
        d.facts.connected = false;
        d.facts.mode = Some(Mode::Socks);
        out.push(("view-socks", d.view()));

        let mut d = daemon(root);
        routed(&mut d);
        d.leak = Some(report(false));
        d.leak_at = sys::now_unix();
        out.push(("view-leak-pass", d.view()));

        let mut d = daemon(root);
        routed(&mut d);
        d.leak = Some(report(true));
        d.leak_at = sys::now_unix();
        out.push(("view-leak-fail", d.view()));

        let mut d = daemon(root);
        routed(&mut d);
        let mut counts = std::collections::BTreeMap::new();
        counts.insert("obfs4".to_string(), 2);
        counts.insert("webtunnel".to_string(), 1);
        let mut preview = bridges::report(&d.host, &counts);
        // The machine the fixture is made on decides availability; fixed here.
        for t in preview.transports.iter_mut() {
            t.available = t.name == "obfs4";
            t.install_hint = if t.available {
                String::new()
            } else {
                bridges::install_hint(&t.name)
            };
        }
        preview.invalid_lines = Some(1);
        d.preview = Some(preview);
        d.pending_bridges = "obfs4 192.0.2.1:443 A\n".into();
        out.push(("view-bridges-preview", d.view()));

        let mut d = daemon(root);
        routed(&mut d);
        let mut counts = std::collections::BTreeMap::new();
        counts.insert("obfs4".to_string(), 2);
        let mut preview = bridges::report(&d.host, &counts);
        for t in preview.transports.iter_mut() {
            t.available = true;
            t.install_hint.clear();
        }
        preview.invalid_lines = Some(0);
        d.preview = Some(preview);
        d.pending_bridges = "obfs4 192.0.2.1:443 A\n".into();
        out.push(("view-bridges-ok", d.view()));
        out
    }

    #[test]
    fn the_qml_fixtures_are_this_daemons_output() {
        let root = sys::create_temp_dir("tm-golden-").unwrap();
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/qml/fixtures");
        let update = std::env::var_os("TORMARCHY_UPDATE_FIXTURES").is_some();
        let mut stale = Vec::new();
        for (name, view) in states(&root) {
            let mut v = view;
            // Time-dependent text is fixed so the files are stable.
            if let Some(s) = v.pointer_mut("/leak/subtitle") {
                if let Some(t) = s.as_str() {
                    *s = Value::String(t.replace("just now", "2 min ago"));
                }
            }
            let text = serde_json::to_string_pretty(&json!({"view": v})).unwrap() + "\n";
            let path = dir.join(format!("{name}.json"));
            if update {
                std::fs::write(&path, &text).unwrap();
            } else if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
                stale.push(name);
            }
        }
        std::fs::remove_dir_all(root).unwrap();
        assert!(stale.is_empty(), "fixtures differ from what the daemon sends ({stale:?}); run TORMARCHY_UPDATE_FIXTURES=1 cargo test");
    }
}
