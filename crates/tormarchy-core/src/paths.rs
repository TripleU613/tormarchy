//! Every file and directory Tormarchy reads or writes, in one place.
//!
//! The system layout is fixed: the polkit policy pins `/usr/local/bin/tormarchy`,
//! tor reads `/etc/tor/torrc.d`, and the panel reads `/run/tormarchy`. Tests
//! build the same layout under a scratch directory with [`Paths::under`], so
//! every function that touches the machine can be exercised without root.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Paths {
    /// Rule state for the unprivileged status reader, wiped on disconnect.
    pub state_dir: PathBuf,
    /// Outside `state_dir`, which disconnect deletes while another command may
    /// hold it.
    pub lock_file: PathBuf,
    /// A fresh token every time panic runs. Panic takes no lock, so a connect
    /// or mode change part-way through putting rules on compares it before and
    /// after, and takes its rules back off if it changed.
    pub panic_marker: PathBuf,
    /// Serialises browser-lock: each rebuilds the whole set of browser rules,
    /// and two at once could each commit a set missing the other's browser.
    pub browser_lock_file: PathBuf,
    pub mode_file: PathBuf,
    pub active_file: PathBuf,
    pub ipv6_state_file: PathBuf,
    pub user_cookie: PathBuf,
    /// Tor's own cookie, wherever this tor build writes it.
    pub tor_cookies: Vec<PathBuf>,

    pub torrc: PathBuf,
    pub torrc_dir: PathBuf,
    pub torrc_base: PathBuf,
    pub torrc_exit: PathBuf,
    pub torrc_exclude: PathBuf,
    pub torrc_bridges: PathBuf,

    /// The saved bridge lines: 0600 root, since which bridges you use is what a
    /// bridge exists to hide.
    pub bridge_stash: PathBuf,
    /// What the stash holds without the lines themselves: transport names and
    /// counts, readable by the panel.
    pub bridge_summary: PathBuf,

    /// Survives a reboot, unlike everything in /run. Only written while the
    /// boot receiver is enabled.
    pub persist_dir: PathBuf,
    pub reconnect_marker: PathBuf,
    pub boot_unit: PathBuf,

    pub bin_dir: PathBuf,
    pub polkit_actions: PathBuf,
    pub polkit_rules: PathBuf,
    pub tmpfiles_dir: PathBuf,
    pub bash_completion: PathBuf,
    pub zsh_completion: PathBuf,

    pub ipv6_conf: PathBuf,
    /// Where nft resolves a `socket cgroupv2` path, and where browser scopes are.
    pub cgroup_root: PathBuf,
    pub proc_root: PathBuf,
}

impl Paths {
    pub fn system() -> Self {
        Self::under(Path::new("/"))
    }

    /// The same layout rooted somewhere else. `/` gives the real system.
    pub fn under(root: &Path) -> Self {
        let p = |rel: &str| root.join(rel.trim_start_matches('/'));
        let state_dir = p("/run/tormarchy");
        let torrc_dir = p("/etc/tor/torrc.d");
        let persist_dir = p("/var/lib/tormarchy");
        Paths {
            lock_file: p("/run/tormarchy.lock"),
            panic_marker: p("/run/tormarchy.panic"),
            browser_lock_file: p("/run/tormarchy-browser.lock"),
            mode_file: state_dir.join("mode"),
            active_file: state_dir.join("active"),
            ipv6_state_file: state_dir.join("ipv6-was"),
            user_cookie: state_dir.join("control.cookie"),
            tor_cookies: vec![
                p("/var/lib/tor/control_auth_cookie"),
                p("/run/tor/control_auth_cookie"),
            ],
            state_dir,
            torrc: p("/etc/tor/torrc"),
            torrc_base: torrc_dir.join("00-tormarchy.conf"),
            torrc_exit: torrc_dir.join("10-tormarchy-exit.conf"),
            torrc_exclude: torrc_dir.join("11-tormarchy-exclude.conf"),
            torrc_bridges: torrc_dir.join("20-tormarchy-bridges.conf"),
            torrc_dir,
            bridge_stash: p("/etc/tormarchy/bridges.stash"),
            bridge_summary: p("/etc/tormarchy/bridges.summary"),
            reconnect_marker: persist_dir.join("reconnect"),
            persist_dir,
            boot_unit: p("/etc/systemd/system/tormarchy-boot.service"),
            bin_dir: p("/usr/local/bin"),
            polkit_actions: p("/usr/share/polkit-1/actions"),
            polkit_rules: p("/etc/polkit-1/rules.d"),
            tmpfiles_dir: p("/etc/tmpfiles.d"),
            bash_completion: p("/usr/share/bash-completion/completions"),
            zsh_completion: p("/usr/share/zsh/site-functions"),
            ipv6_conf: p("/proc/sys/net/ipv6/conf"),
            cgroup_root: p("/sys/fs/cgroup"),
            proc_root: p("/proc"),
        }
    }

    /// Where to look for a control cookie, best first. The published copy comes
    /// first because it is the only one that works without a fresh login: group
    /// membership reaches a process at login and never after.
    pub fn cookie_candidates(&self) -> Vec<PathBuf> {
        let mut out = vec![self.user_cookie.clone()];
        out.extend(self.tor_cookies.iter().cloned());
        out
    }

    pub fn helper(&self) -> PathBuf {
        self.bin_dir.join("tormarchy")
    }

    pub fn daemon(&self) -> PathBuf {
        self.bin_dir.join("tormarchyd")
    }
}

/// The installed helper, by its fixed path. The polkit policy pins exactly this
/// file, so it is what every escalation re-executes.
pub const HELPER_PATH: &str = "/usr/local/bin/tormarchy";
