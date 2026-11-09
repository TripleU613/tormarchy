//! Running other programs and touching files, with the guarantees the rest of
//! the crate relies on: a fixed PATH, bounded output, deadlines, and writes that
//! never follow a planted symlink.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// Every external program is found on this PATH and nowhere else. The helper
/// runs as root, and inheriting a caller's PATH would let the first writable
/// directory on it decide what "nft" means.
pub const FIXED_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// What a finished program left behind.
#[derive(Debug, Clone, Default)]
pub struct Output {
    /// None when it was killed for running past its deadline, or never ran.
    pub status: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// More than the limit arrived on stdout; what is kept is cut there.
    pub overflowed: bool,
    pub timed_out: bool,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.status == Some(0)
    }
    pub fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
    pub fn stderr_str(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

/// How to run something.
pub struct Run<'a> {
    pub program: &'a str,
    pub args: Vec<String>,
    pub stdin: Option<Vec<u8>>,
    pub timeout: Duration,
    pub stdout_limit: usize,
    pub stderr_limit: usize,
    pub env: Vec<(String, String)>,
    pub clear_env: bool,
    /// Keep the caller's own PATH. Only for programs run as the user that live
    /// where only the user's PATH reaches -- Omarchy's own commands -- and never
    /// in anything that runs as root.
    pub user_path: bool,
}

impl<'a> Run<'a> {
    pub fn new<I, S>(program: &'a str, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Run {
            program,
            args: args.into_iter().map(|a| a.as_ref().to_string()).collect(),
            stdin: None,
            timeout: Duration::from_secs(30),
            stdout_limit: 1 << 20,
            stderr_limit: 16 << 10,
            env: Vec::new(),
            clear_env: false,
            user_path: false,
        }
    }
    pub fn user_path(mut self) -> Self {
        self.user_path = true;
        self
    }
    pub fn stdin(mut self, data: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(data.into());
        self
    }
    pub fn timeout(mut self, t: Duration) -> Self {
        self.timeout = t;
        self
    }
    pub fn limit(mut self, bytes: usize) -> Self {
        self.stdout_limit = bytes;
        self
    }
    pub fn env(mut self, k: &str, v: &str) -> Self {
        self.env.push((k.into(), v.into()));
        self
    }

    /// Runs it to completion or to its deadline, whichever is first. Output past
    /// a limit is never retained: the readers stop keeping bytes there, and the
    /// child is killed if stdout overflows, so an endless producer costs nothing.
    pub fn run(self) -> Output {
        let program = if self.user_path || self.program.contains('/') {
            PathBuf::from(self.program)
        } else {
            // Found on the fixed PATH, whatever the caller's says.
            which(self.program)
                .unwrap_or_else(|| PathBuf::from(format!("/nonexistent/{}", self.program)))
        };
        let mut cmd = Command::new(&program);
        cmd.args(&self.args);
        if self.clear_env {
            cmd.env_clear();
        }
        if !self.user_path {
            cmd.env("PATH", FIXED_PATH);
        }
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        cmd.stdin(if self.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        // A process group of its own, so a deadline takes descendants with it.
        unsafe_process_group(&mut cmd);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return Output {
                    status: if e.kind() == io::ErrorKind::NotFound {
                        Some(127)
                    } else {
                        Some(126)
                    },
                    stderr: format!("{}: {}", self.program, e).into_bytes(),
                    ..Default::default()
                }
            }
        };
        let pid = child.id() as i32;

        if let Some(data) = self.stdin {
            if let Some(mut sink) = child.stdin.take() {
                thread::spawn(move || {
                    let _ = sink.write_all(&data);
                });
            }
        }
        let (otx, orx) = mpsc::channel();
        let out = child.stdout.take().expect("piped");
        let olimit = self.stdout_limit;
        thread::spawn(move || {
            let _ = otx.send(read_capped(out, olimit, Some(pid)));
        });
        let (etx, erx) = mpsc::channel();
        let err = child.stderr.take().expect("piped");
        let elimit = self.stderr_limit;
        thread::spawn(move || {
            let _ = etx.send(read_capped(err, elimit, None));
        });

        let deadline = Instant::now() + self.timeout;
        let mut timed_out = false;
        let status = loop {
            match child.try_wait() {
                Ok(Some(st)) => break st.code(),
                Ok(None) => {}
                Err(_) => break None,
            }
            if Instant::now() >= deadline {
                timed_out = true;
                kill_group(pid);
                let _ = child.wait();
                break None;
            }
            thread::sleep(Duration::from_millis(10));
        };
        // A descendant holding the pipes open must not hold this up either.
        kill_group(pid);
        let (stdout, overflowed) = orx.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
        let (stderr, _) = erx.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
        Output {
            status,
            stdout,
            stderr,
            overflowed,
            timed_out,
        }
    }
}

fn unsafe_process_group(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
}

pub fn kill_group(pid: i32) {
    if pid > 0 {
        // SAFETY: plain signal delivery to a process group this code created.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
}

/// Reads to EOF keeping at most `limit` bytes. One byte past the limit marks
/// the read as overflowed; with `kill`, that group is killed then and there.
fn read_capped(mut r: impl Read, limit: usize, kill: Option<i32>) -> (Vec<u8>, bool) {
    let mut kept = Vec::new();
    let mut buf = [0u8; 8192];
    let mut overflowed = false;
    loop {
        match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let room = limit.saturating_sub(kept.len());
                if n > room {
                    kept.extend_from_slice(&buf[..room]);
                    overflowed = true;
                    if let Some(pid) = kill {
                        kill_group(pid);
                    }
                    // Keep draining, discarding, so the writer is not blocked.
                    while let Ok(m) = r.read(&mut buf) {
                        if m == 0 {
                            break;
                        }
                    }
                    break;
                }
                kept.extend_from_slice(&buf[..n]);
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    (kept, overflowed)
}

/// Whether a program can be found on the fixed PATH, and where.
pub fn which(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        let p = PathBuf::from(name);
        return is_executable(&p).then_some(p);
    }
    for dir in FIXED_PATH.split(':') {
        let p = Path::new(dir).join(name);
        if is_executable(&p) {
            return Some(p);
        }
    }
    None
}

/// Like [`which`], over the caller's own PATH: for programs the user names,
/// such as a browser, which run as that user and never as root.
pub fn which_user(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        let p = PathBuf::from(name);
        return is_executable(&p).then_some(p);
    }
    let path = std::env::var("PATH").unwrap_or_else(|_| FIXED_PATH.into());
    path.split(':')
        .filter(|d| !d.is_empty())
        .map(|d| Path::new(d).join(name))
        .find(|p| is_executable(p))
}

fn is_executable(p: &Path) -> bool {
    fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

pub fn euid() -> u32 {
    // SAFETY: no preconditions.
    unsafe { libc::geteuid() }
}

pub fn is_root() -> bool {
    euid() == 0
}

/// At most `limit` bytes of a file, refused rather than cut if there is more,
/// opened without following a symlink and without blocking on a fifo.
pub fn read_bounded(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    let f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)?;
    if !f.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a regular file",
        ));
    }
    let mut out = Vec::new();
    f.take(limit as u64 + 1).read_to_end(&mut out)?;
    if out.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file is larger than expected",
        ));
    }
    Ok(out)
}

pub fn read_bounded_string(path: &Path, limit: usize) -> Option<String> {
    read_bounded(path, limit)
        .ok()
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// Replaces `path` in one step: a fresh file created exclusively beside it,
/// written, given `mode`, then renamed over. A reader sees the old content or
/// the new, never half, and the write can never follow a symlink.
pub fn write_atomic(path: &Path, content: &[u8], mode: u32) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path.file_name().and_then(OsStr::to_str).unwrap_or("file");
    let tmp = create_exclusive_temp(dir, &format!(".{name}."), mode)?;
    let result = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&tmp)?;
        f.write_all(content)?;
        f.sync_all()?;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// A new, empty file with an unpredictable name, created with O_EXCL.
pub fn create_exclusive_temp(dir: &Path, prefix: &str, mode: u32) -> io::Result<PathBuf> {
    for _ in 0..64 {
        let candidate = dir.join(format!("{prefix}{}", random_token(12)));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&candidate)
        {
            Ok(_) => return Ok(candidate),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not create a temporary file",
    ))
}

/// A private temporary directory, 0700, created exclusively.
pub fn create_temp_dir(prefix: &str) -> io::Result<PathBuf> {
    let base = std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp".into());
    for _ in 0..64 {
        let candidate = base.join(format!("{prefix}{}", random_token(12)));
        match fs::DirBuilder::new().mode(0o700).create(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not create a temporary directory",
    ))
}

/// Unpredictable characters from the kernel's random source.
pub fn random_token(len: usize) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut bytes = vec![0u8; len];
    if File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .is_err()
    {
        // SAFETY: getrandom writes at most `len` bytes into the buffer.
        unsafe {
            libc::getrandom(bytes.as_mut_ptr() as *mut libc::c_void, len, 0);
        }
    }
    bytes
        .iter()
        .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
        .collect()
}

/// `install -d -m mode`: create if missing, then set the mode either way.
pub fn ensure_dir(path: &Path, mode: u32) -> io::Result<()> {
    match fs::DirBuilder::new()
        .recursive(true)
        .mode(mode)
        .create(path)
    {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "exists and is not a directory",
        ));
    }
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

pub fn remove_file_quiet(path: &Path) {
    let _ = fs::remove_file(path);
}

/// chown by numeric ids, never following a symlink.
pub fn lchown(path: &Path, uid: u32, gid: u32) -> io::Result<()> {
    std::os::unix::fs::lchown(path, Some(uid), Some(gid))
}

/// A user's uid, gid and home, from the passwd database.
pub fn user_info(name: &str) -> Option<(u32, u32, PathBuf)> {
    let out = Run::new("getent", ["passwd", name])
        .timeout(Duration::from_secs(5))
        .run();
    if !out.ok() {
        return None;
    }
    let line = out.stdout_str();
    let f: Vec<&str> = line.trim_end().split(':').collect();
    if f.len() < 7 || f[0] != name {
        return None;
    }
    Some((f[2].parse().ok()?, f[3].parse().ok()?, PathBuf::from(f[5])))
}

pub fn user_name(uid: u32) -> Option<String> {
    let out = Run::new("getent", ["passwd", &uid.to_string()])
        .timeout(Duration::from_secs(5))
        .run();
    if !out.ok() {
        return None;
    }
    out.stdout_str()
        .split(':')
        .next()
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

pub fn user_groups(name: &str) -> Vec<String> {
    let out = Run::new("id", ["-nG", name])
        .timeout(Duration::from_secs(5))
        .run();
    out.stdout_str()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

pub fn group_exists(name: &str) -> bool {
    Run::new("getent", ["group", name])
        .timeout(Duration::from_secs(5))
        .run()
        .ok()
}

/// A valid Unix account name, as useradd accepts them.
pub fn valid_user_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c == '_' => {}
        _ => return false,
    }
    name.len() <= 32
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// UTC, as `2026-11-09T16:38:20Z`.
pub fn utc_timestamp(secs: u64) -> String {
    let (y, mo, d, h, mi, s) = civil_from_unix(secs as i64);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

fn civil_from_unix(t: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = t.div_euclid(86_400);
    let rem = t.rem_euclid(86_400);
    let (h, mi, s) = (
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
        (rem % 60) as u32,
    );
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if mo <= 2 { y + 1 } else { y }, mo, d, h, mi, s)
}

/// Seconds since the epoch for `YYYY-MM-DDTHH:MM:SS[.frac]` in UTC.
pub fn parse_utc(s: &str) -> Option<i64> {
    let s = s.split('.').next()?;
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>());
    let (y, m, day) = (d.next()?.ok()?, d.next()?.ok()?, d.next()?.ok()?);
    let mut t = time.split(':').map(|x| x.parse::<i64>());
    let (h, mi, sec) = (t.next()?.ok()?, t.next()?.ok()?, t.next()?.ok()?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    // Days from civil, the inverse of the above.
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2.rem_euclid(400);
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + sec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_round_trip() {
        for t in [0u64, 951_782_400, 1_794_233_500, 4_102_444_800] {
            assert_eq!(
                parse_utc(utc_timestamp(t).trim_end_matches('Z')),
                Some(t as i64)
            );
        }
        assert_eq!(utc_timestamp(1_794_242_300), "2026-11-09T16:38:20Z");
        assert_eq!(parse_utc("2026-11-09T16:38:20.123456"), Some(1_794_242_300));
        assert_eq!(parse_utc("garbage"), None);
    }

    #[test]
    fn output_past_the_limit_is_cut_and_marked() {
        let out = Run::new("sh", ["-c", "yes | head -c 100000"])
            .limit(1000)
            .run();
        assert!(out.overflowed);
        assert_eq!(out.stdout.len(), 1000);
    }

    #[test]
    fn a_deadline_kills_the_whole_group() {
        let start = Instant::now();
        let out = Run::new("sh", ["-c", "sleep 30 & sleep 30"])
            .timeout(Duration::from_millis(200))
            .run();
        assert!(out.timed_out);
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_missing_program_is_127() {
        assert_eq!(
            Run::new("tormarchy-no-such-program", Vec::<String>::new())
                .run()
                .status,
            Some(127)
        );
    }

    #[test]
    fn bounded_reads_refuse_rather_than_cut() {
        let dir = create_temp_dir("tm-test-").unwrap();
        let f = dir.join("f");
        fs::write(&f, vec![b'a'; 11]).unwrap();
        assert!(read_bounded(&f, 10).is_err());
        assert_eq!(read_bounded(&f, 11).unwrap().len(), 11);
        let link = dir.join("l");
        std::os::unix::fs::symlink(&f, &link).unwrap();
        assert!(
            read_bounded(&link, 100).is_err(),
            "a symlink is never followed"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn atomic_writes_replace_and_never_follow_links() {
        let dir = create_temp_dir("tm-test-").unwrap();
        let target = dir.join("victim");
        fs::write(&target, b"keep").unwrap();
        let path = dir.join("state");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        write_atomic(&path, b"new", 0o644).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"keep");
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert!(!fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn user_names() {
        assert!(valid_user_name("alice"));
        assert!(valid_user_name("_svc-1"));
        assert!(!valid_user_name("Alice"));
        assert!(!valid_user_name("a/b"));
        assert!(!valid_user_name(""));
        assert!(!valid_user_name("1abc"));
    }
}
