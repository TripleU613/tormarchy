//! Tormarchy's system layer: everything the `tormarchy` helper and the
//! `tormarchyd` daemon share.

pub mod bridges;
pub mod control;
pub mod doctor;
pub mod exits;
pub mod firewall;
pub mod host;
pub mod menu;
pub mod ops;
pub mod paths;
pub mod socks;
pub mod state;
pub mod status;
pub mod sys;
pub mod tor;
pub mod view;

use std::fmt;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// How much goes through Tor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// All WAN through Tor; the local network still reachable.
    Lan,
    /// Everything through Tor, the local network dropped too.
    Strict,
    /// Nothing system-wide; apps opt in to the SOCKS proxy.
    Socks,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Lan, Mode::Strict, Mode::Socks];
    /// The order the panel draws them: most protective first.
    pub const DISPLAY_ORDER: [Mode; 3] = [Mode::Strict, Mode::Lan, Mode::Socks];

    pub fn parse(s: &str) -> Option<Mode> {
        match s.trim() {
            "lan" => Some(Mode::Lan),
            "strict" => Some(Mode::Strict),
            "socks" => Some(Mode::Socks),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Lan => "lan",
            Mode::Strict => "strict",
            Mode::Socks => "socks",
        }
    }

    pub fn routed(self) -> bool {
        self != Mode::Socks
    }

    pub fn label(self) -> &'static str {
        match self {
            Mode::Lan => "Standard",
            Mode::Strict => "Maximum",
            Mode::Socks => "Browser only",
        }
    }

    pub fn tooltip(self) -> &'static str {
        match self {
            Mode::Lan => "All internet traffic through Tor. Local network still reachable.",
            Mode::Strict => "All traffic through Tor. Local network dropped too.",
            Mode::Socks => "Nothing is routed system-wide. Any app that speaks SOCKS can opt in on 127.0.0.1:9050 — a browser is just the usual one.",
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A failure meant for a person: printed as `tormarchy: <message>`, exit 1.
#[derive(Debug, Clone, PartialEq)]
pub struct Fail(pub String);

impl fmt::Display for Fail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<T: Into<String>> From<T> for Fail {
    fn from(s: T) -> Self {
        Fail(s.into())
    }
}

pub type Res<T = ()> = Result<T, Fail>;

#[macro_export]
macro_rules! fail {
    ($($t:tt)*) => { return Err($crate::Fail(format!($($t)*))) };
}
