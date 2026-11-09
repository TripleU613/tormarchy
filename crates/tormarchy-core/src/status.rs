//! Where things stand, in facts and in words. The command-line `status` and the
//! daemon build the same [`Facts`] and describe them the same way, so the bar's
//! tooltip and the terminal can never disagree.

use serde::Serialize;

use crate::host::{Host, SOCKS_PORT};
use crate::tor::{self, Hop};
use crate::{bridges, exits, state, sys, Mode};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    pub installed: bool,
    pub tor_running: bool,
    /// Rules are on.
    pub connected: bool,
    /// The control port answered. Without it the bootstrap figure is a
    /// placeholder 0, not a measurement.
    pub control_port: bool,
    pub bootstrap: u8,
    pub mode: Option<Mode>,
    pub hops: Vec<Hop>,
    pub exit_ip: String,
    pub circuit_age: u64,
    pub bytes_read: u64,
    pub bytes_written: u64,
    pub requested_exit: Option<String>,
    pub excluded_exits: Vec<String>,
    pub bridges: String,
}

impl Facts {
    pub fn exit_country(&self) -> &str {
        match self.hops.last().map(|h| h.cc.as_str()) {
            Some("xx") | None => "",
            Some(cc) => cc,
        }
    }

    /// The mode in force: the recorded one, or lan when rules are on with none
    /// recorded.
    pub fn mode_or_default(&self) -> Option<Mode> {
        self.mode.or(if self.connected {
            Some(Mode::Lan)
        } else {
            None
        })
    }

    /// The state line, in order of what most needs saying.
    pub fn status_text(&self) -> String {
        let mode = self.mode_or_default();
        let settling = self.control_port && self.bootstrap < 100;
        if !self.installed {
            "Tor helpers are not installed".into()
        } else if self.connected && !self.tor_running {
            // Before any "through Tor" wording: nothing is going through it.
            "Kill switch holding: tor is not running, so the network is blocked".into()
        } else if self.connected && settling {
            format!(
                "Reconnecting to Tor at {}%; traffic waits until it is back",
                self.bootstrap
            )
        } else if mode == Some(Mode::Socks) && self.tor_running {
            format!("Browser-only: SOCKS proxy on 127.0.0.1:{SOCKS_PORT}")
        } else if self.connected && !self.exit_country().is_empty() {
            format!(
                "All traffic through Tor via {}",
                self.exit_country().to_uppercase()
            )
        } else if self.connected {
            "All traffic through Tor".into()
        } else if self.tor_running && settling {
            format!("Bootstrapping at {}%", self.bootstrap)
        } else if self.tor_running {
            "Tor is running but traffic is not routed through it".into()
        } else {
            "Disconnected".into()
        }
    }

    pub fn warnings(&self) -> Vec<String> {
        let mut w = Vec::new();
        // Rules on, no Tor behind them: not leaking, going nowhere -- and the
        // user needs to know why the machine went quiet.
        if self.connected && !self.tor_running {
            w.push("Tor has stopped, so the kill switch is holding the network shut. Turn Tormarchy off to get it back.".into());
        }
        if self.connected
            && self.tor_running
            && self.control_port
            && self.mode != Some(Mode::Socks)
            && self.bootstrap < 100
        {
            w.push(format!(
                "Tor is still bootstrapping at {}%.",
                self.bootstrap
            ));
        }
        w
    }
}

/// Everything but what tor itself reports.
pub fn local_facts(host: &dyn Host) -> Facts {
    let paths = host.paths();
    Facts {
        installed: paths.torrc_base.exists() && host.tor_installed(),
        tor_running: host.tor_running(),
        connected: state::rules_present(host),
        mode: state::current_mode(paths),
        requested_exit: exits::requested(paths),
        excluded_exits: exits::excluded(paths),
        bridges: bridges::state(paths).into(),
        ..Default::default()
    }
}

/// The facts, asking tor over one-shot sessions.
pub fn gather(host: &dyn Host, ask: &mut dyn tor::Ask) -> Facts {
    let mut f = local_facts(host);
    if f.tor_running {
        if let Ok(v) = tor::vitals(ask) {
            f.control_port = true;
            f.bootstrap = v.bootstrap;
            f.bytes_read = v.bytes_read;
            f.bytes_written = v.bytes_written;
        }
        if f.bootstrap >= 100 {
            if let Ok(Some(p)) = tor::current_path(ask) {
                f.circuit_age = p.age_seconds(sys::now_unix());
                f.exit_ip = p.exit_ip().to_string();
                f.hops = p.hops;
            }
        }
    }
    f
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusJson<'a> {
    installed: bool,
    tor_running: bool,
    connected: bool,
    bootstrap: u8,
    control_port: bool,
    bridges: &'a str,
    mode: &'a str,
    exit_ip: &'a str,
    exit_country: &'a str,
    requested_exit: &'a str,
    excluded_exits: &'a [String],
    circuit_age_sec: u64,
    path: Vec<&'a str>,
    hops: &'a [Hop],
    bytes_read: u64,
    bytes_written: u64,
    status_text: String,
    warnings: Vec<String>,
}

/// `status --json`, the same shape the panel once read.
pub fn to_json(f: &Facts) -> String {
    let j = StatusJson {
        installed: f.installed,
        tor_running: f.tor_running,
        connected: f.connected,
        bootstrap: f.bootstrap,
        control_port: f.control_port,
        bridges: &f.bridges,
        mode: f.mode_or_default().map(Mode::as_str).unwrap_or(""),
        exit_ip: &f.exit_ip,
        exit_country: f.exit_country(),
        requested_exit: f.requested_exit.as_deref().unwrap_or(""),
        excluded_exits: &f.excluded_exits,
        circuit_age_sec: f.circuit_age,
        path: f.hops.iter().map(|h| h.cc.as_str()).collect(),
        hops: &f.hops,
        bytes_read: f.bytes_read,
        bytes_written: f.bytes_written,
        status_text: f.status_text(),
        warnings: f.warnings(),
    };
    serde_json::to_string_pretty(&j).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Facts {
        Facts {
            installed: true,
            tor_running: true,
            control_port: true,
            bootstrap: 100,
            ..Default::default()
        }
    }

    #[test]
    fn the_kill_switch_holding_is_said_before_anything_else() {
        let f = Facts {
            connected: true,
            tor_running: false,
            mode: Some(Mode::Lan),
            ..facts()
        };
        assert!(f.status_text().starts_with("Kill switch holding"));
        assert!(f.warnings()[0].contains("kill switch"));
    }

    #[test]
    fn an_unanswered_control_port_is_not_bootstrapping_at_zero() {
        let f = Facts {
            control_port: false,
            bootstrap: 0,
            ..facts()
        };
        assert_eq!(
            f.status_text(),
            "Tor is running but traffic is not routed through it"
        );
        let f = Facts {
            bootstrap: 40,
            ..facts()
        };
        assert_eq!(f.status_text(), "Bootstrapping at 40%");
    }

    #[test]
    fn the_exit_country_is_named_once_known() {
        let mut f = Facts {
            connected: true,
            mode: Some(Mode::Strict),
            ..facts()
        };
        f.hops = vec![
            Hop {
                cc: "se".into(),
                ..Default::default()
            },
            Hop {
                cc: "de".into(),
                ..Default::default()
            },
        ];
        assert_eq!(f.status_text(), "All traffic through Tor via DE");
        f.hops.last_mut().unwrap().cc = "xx".into();
        assert_eq!(f.status_text(), "All traffic through Tor");
    }

    #[test]
    fn browser_only_says_where_the_proxy_is() {
        let f = Facts {
            mode: Some(Mode::Socks),
            ..facts()
        };
        assert_eq!(
            f.status_text(),
            "Browser-only: SOCKS proxy on 127.0.0.1:9050"
        );
        assert!(f.warnings().is_empty());
    }

    #[test]
    fn the_json_keeps_its_shape() {
        let v: serde_json::Value = serde_json::from_str(&to_json(&facts())).unwrap();
        for key in [
            "installed",
            "torRunning",
            "connected",
            "bootstrap",
            "controlPort",
            "bridges",
            "mode",
            "exitIp",
            "exitCountry",
            "requestedExit",
            "excludedExits",
            "circuitAgeSec",
            "path",
            "hops",
            "bytesRead",
            "bytesWritten",
            "statusText",
            "warnings",
        ] {
            assert!(v.get(key).is_some(), "missing {key}");
        }
    }
}
