//! What survives between commands: the mode and whether rules are on (for the
//! unprivileged reader), the lock that serialises routing changes, panic's
//! token, and the boot receiver's marker.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::thread;
use std::time::{Duration, Instant};

use crate::firewall;
use crate::host::Host;
use crate::paths::Paths;
use crate::sys;
use crate::{Fail, Mode, Res};

pub fn current_mode(paths: &Paths) -> Option<Mode> {
    sys::read_bounded_string(&paths.mode_file, 64).and_then(|s| Mode::parse(&s))
}

/// "Connected" means a ruleset is installed, not merely that tor is up. As root
/// nft is asked; unprivileged, the state file is all there is.
pub fn rules_present(host: &dyn Host) -> bool {
    if sys::is_root() {
        firewall::table_exists(host, firewall::NFT_TABLE)
    } else {
        host.paths().active_file.exists()
    }
}

/// Both files through an exclusive temp and a rename, readable by the poller.
pub fn write_state(paths: &Paths, mode: Mode, active: bool) -> Res {
    sys::ensure_dir(&paths.state_dir, 0o755).map_err(|e| {
        Fail(format!(
            "could not create {}: {e}",
            paths.state_dir.display()
        ))
    })?;
    sys::write_atomic(&paths.mode_file, format!("{mode}\n").as_bytes(), 0o644)
        .map_err(|e| Fail(e.to_string()))?;
    if active {
        sys::write_atomic(&paths.active_file, b"", 0o644).map_err(|e| Fail(e.to_string()))?;
    } else {
        sys::remove_file_quiet(&paths.active_file);
    }
    Ok(())
}

pub fn clear_state(paths: &Paths) {
    let _ = std::fs::remove_dir_all(&paths.state_dir);
}

/// Serialises the commands that change routing. Held for the moment of change
/// only, never across a bootstrap wait, so a disconnect can always get in to
/// cancel a connect still waiting for tor -- and one arriving while a connect
/// applies its rules waits for it and then takes them down, rather than racing.
pub struct Lock {
    _file: Option<File>,
}

impl Lock {
    pub fn take(paths: &Paths, wait: Duration) -> Res<Lock> {
        Self::take_file(
            &paths.lock_file,
            wait,
            "another tormarchy command is still changing the network; try again",
        )
    }

    pub fn take_browser(paths: &Paths) -> Res<Lock> {
        Self::take_file(
            &paths.browser_lock_file,
            Duration::from_secs(10),
            "another browser is being locked right now; try again",
        )
    }

    fn take_file(path: &std::path::Path, wait: Duration, busy: &str) -> Res<Lock> {
        if !sys::is_root() {
            return Ok(Lock { _file: None });
        }
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map_err(|e| Fail(format!("could not open the lock {}: {e}", path.display())))?;
        let deadline = Instant::now() + wait;
        loop {
            // SAFETY: flock on a descriptor this function owns.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                return Ok(Lock { _file: Some(file) });
            }
            if Instant::now() >= deadline {
                return Err(Fail(busy.into()));
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
}

pub fn panic_token(paths: &Paths) -> String {
    sys::read_bounded_string(&paths.panic_marker, 256).unwrap_or_default()
}

/// Panic writes this before it flushes anything. Panic takes no lock, so it is
/// how a connect or mode change applying rules right now learns to take them
/// back off.
pub fn mark_panic(paths: &Paths) -> bool {
    let token = format!("{}.{}\n", sys::now_unix(), sys::random_token(16));
    sys::write_atomic(&paths.panic_marker, token.as_bytes(), 0o600).is_ok()
}

/// The boot receiver reads this to decide whether to reconnect, and in which
/// mode. Only written while the receiver is enabled.
pub fn remember_connected(paths: &Paths, mode: Mode) {
    if !paths.boot_unit.exists() || !mode.routed() {
        return;
    }
    if sys::ensure_dir(&paths.persist_dir, 0o755).is_ok() {
        let _ = sys::write_atomic(
            &paths.reconnect_marker,
            format!("{mode}\n").as_bytes(),
            0o644,
        );
    }
}

pub fn remembered_mode(paths: &Paths) -> Option<Mode> {
    sys::read_bounded_string(&paths.reconnect_marker, 16)
        .and_then(|s| Mode::parse(&s))
        .filter(|m| m.routed())
}

pub fn forget_connected(paths: &Paths) {
    sys::remove_file_quiet(&paths.reconnect_marker);
}
