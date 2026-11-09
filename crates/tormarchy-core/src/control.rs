//! Tor's control protocol, as far as Tormarchy speaks it.
//!
//! Two clients over one parser. [`query`] is a session per call -- authenticate,
//! send, QUIT, read to the end -- for the command-line tool. [`Conn`] is one
//! long-lived session for the daemon, subscribed to events, so the panel is told
//! when something changes instead of asking every few seconds.
//!
//! Every read is bounded twice: by bytes, so an endpoint that answers without
//! stopping cannot grow the reader, and by a deadline over the whole operation,
//! so one that trickles a byte at a time cannot hold it either.

use std::collections::HashMap;
use std::fmt;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crate::sys;

pub const CONTROL_PORT: u16 = 9051;
/// The most a reply to anything the panel polls may occupy. Far above any real
/// one: the largest is a full circuit-status listing, a few hundred bytes per
/// circuit.
pub const REPLY_LIMIT: usize = 256 * 1024;
/// The one reply that is not small: the whole consensus, about 200 bytes a
/// relay, so around 1.5 MB with today's network.
pub const CONSENSUS_LIMIT: usize = 32 * 1024 * 1024;
/// A single line, data lines included.
const LINE_LIMIT: usize = 64 * 1024;

#[derive(Debug)]
pub enum Error {
    /// Nothing listening, or no cookie that authenticates.
    Unreachable(String),
    /// More than the limit arrived.
    TooLarge,
    /// The deadline passed before the reply ended.
    Timeout,
    /// A reply that does not follow the protocol.
    Protocol(String),
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unreachable(why) => write!(f, "could not reach tor's control port: {why}"),
            Error::TooLarge => write!(f, "tor's reply was larger than expected"),
            Error::Timeout => write!(f, "tor's control port did not answer in time"),
            Error::Protocol(why) => write!(f, "unexpected reply from tor: {why}"),
            Error::Io(e) => write!(f, "control port: {e}"),
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => Error::Timeout,
            _ => Error::Io(e),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// One line of a reply: its text after the status code, and the data block a
/// `+` line carries.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Line {
    pub text: String,
    pub data: Option<String>,
}

/// A complete reply or event: every line up to and including the one whose
/// separator is a space.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reply {
    pub code: u16,
    pub lines: Vec<Line>,
}

impl Reply {
    pub fn is_ok(&self) -> bool {
        (200..300).contains(&self.code)
    }
    pub fn first_text(&self) -> &str {
        self.lines.first().map(|l| l.text.as_str()).unwrap_or("")
    }
    /// `key=value` pairs, as GETINFO answers them. A data block is its value.
    pub fn values(&self) -> HashMap<String, String> {
        let mut out = HashMap::new();
        for line in &self.lines {
            if let Some((k, v)) = line.text.split_once('=') {
                let value = match &line.data {
                    Some(d) => d.clone(),
                    None => v.to_string(),
                };
                out.insert(k.to_string(), value);
            }
        }
        out
    }
}

/// Reads reply lines from a byte stream, enforcing a byte budget and a deadline.
pub struct Reader<R: Read> {
    inner: BufReader<R>,
    budget: usize,
    deadline: Option<Instant>,
    set_timeout: Option<Box<dyn Fn(Option<Duration>) -> io::Result<()> + Send>>,
}

impl<R: Read> Reader<R> {
    pub fn new(r: R, budget: usize) -> Self {
        Reader {
            inner: BufReader::new(r),
            budget,
            deadline: None,
            set_timeout: None,
        }
    }

    fn line(&mut self) -> Result<Option<String>> {
        let mut buf = Vec::new();
        loop {
            if let (Some(deadline), Some(set)) = (self.deadline, &self.set_timeout) {
                let left = deadline
                    .checked_duration_since(Instant::now())
                    .ok_or(Error::Timeout)?;
                set(Some(left.max(Duration::from_millis(1))))?;
            }
            let available = match self.inner.fill_buf() {
                Ok(b) => b,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            };
            if available.is_empty() {
                return if buf.is_empty() {
                    Ok(None)
                } else {
                    Err(Error::Protocol("reply cut off".into()))
                };
            }
            let (used, done) = match available.iter().position(|b| *b == b'\n') {
                Some(i) => (i + 1, true),
                None => (available.len(), false),
            };
            // Over the whole reply's budget, or over one line's: either way the
            // bytes are refused before they are kept.
            if used > self.budget || buf.len() + used > LINE_LIMIT {
                return Err(Error::TooLarge);
            }
            buf.extend_from_slice(&available[..used]);
            self.budget -= used;
            self.inner.consume(used);
            if done {
                while matches!(buf.last(), Some(b'\n' | b'\r')) {
                    buf.pop();
                }
                return Ok(Some(String::from_utf8_lossy(&buf).into_owned()));
            }
        }
    }

    /// The next whole reply, or None at a clean end of stream.
    pub fn reply(&mut self) -> Result<Option<Reply>> {
        let mut reply = Reply::default();
        loop {
            let Some(raw) = self.line()? else {
                return if reply.lines.is_empty() {
                    Ok(None)
                } else {
                    Err(Error::Protocol("reply cut off".into()))
                };
            };
            if raw.len() < 4 || !raw.is_char_boundary(3) || !raw.is_char_boundary(4) {
                return Err(Error::Protocol(format!("short line {raw:?}")));
            }
            let code: u16 = raw[..3]
                .parse()
                .map_err(|_| Error::Protocol(format!("bad status in {raw:?}")))?;
            let sep = raw.as_bytes()[3];
            let text = raw[4..].to_string();
            reply.code = code;
            match sep {
                b' ' => {
                    reply.lines.push(Line { text, data: None });
                    return Ok(Some(reply));
                }
                b'-' => reply.lines.push(Line { text, data: None }),
                b'+' => {
                    let mut data = String::new();
                    loop {
                        let Some(mut d) = self.line()? else {
                            return Err(Error::Protocol("data block cut off".into()));
                        };
                        if d == "." {
                            break;
                        }
                        if d.starts_with("..") {
                            d.remove(0);
                        }
                        data.push_str(&d);
                        data.push('\n');
                    }
                    reply.lines.push(Line {
                        text,
                        data: Some(data),
                    });
                }
                _ => return Err(Error::Protocol(format!("bad separator in {raw:?}"))),
            }
        }
    }
}

/// How to reach tor: which port, and which cookies to try.
#[derive(Clone, Debug)]
pub struct Endpoint {
    pub port: u16,
    pub cookies: Vec<PathBuf>,
}

impl Endpoint {
    pub fn new(cookies: Vec<PathBuf>) -> Self {
        Endpoint {
            port: CONTROL_PORT,
            cookies,
        }
    }

    fn addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], self.port))
    }

    /// The cookies this process can read, in order.
    fn readable_cookies(&self) -> Vec<String> {
        self.cookies
            .iter()
            .filter_map(|p| sys::read_bounded(p, 64).ok())
            .filter(|c| !c.is_empty())
            .map(|c| c.iter().map(|b| format!("{b:02x}")).collect())
            .collect()
    }

    /// A connected, authenticated stream. Every readable cookie is tried in
    /// turn, each on a connection of its own, since tor closes the session after
    /// a failed AUTHENTICATE. The published copy can be older than tor's current
    /// cookie if tor restarted with nobody around to republish it, and then the
    /// original still gets through.
    pub fn open(&self, timeout: Duration) -> Result<(TcpStream, Reader<TcpStream>)> {
        let cookies = self.readable_cookies();
        if cookies.is_empty() {
            return Err(Error::Unreachable("no readable control cookie".into()));
        }
        let mut last = Error::Unreachable("no cookie authenticated".into());
        for cookie in cookies {
            let stream =
                match TcpStream::connect_timeout(&self.addr(), timeout.min(Duration::from_secs(3)))
                {
                    Ok(s) => s,
                    Err(e) => return Err(Error::Unreachable(e.to_string())),
                };
            stream.set_read_timeout(Some(Duration::from_secs(3)))?;
            stream.set_write_timeout(Some(Duration::from_secs(3)))?;
            let mut writer = stream.try_clone()?;
            let mut reader = Reader::new(stream.try_clone()?, 4096);
            writer.write_all(format!("AUTHENTICATE {cookie}\r\n").as_bytes())?;
            match reader.reply() {
                Ok(Some(r)) if r.is_ok() => return Ok((writer, Reader::new(stream, 0))),
                Ok(_) => last = Error::Unreachable("tor refused every control cookie".into()),
                Err(e) => last = e,
            }
            let _ = writer.shutdown(Shutdown::Both);
        }
        Err(last)
    }
}

/// One session: every command, then QUIT, then every reply up to it. Replies
/// arrive in request order, so the result lines up with `commands`.
pub fn query(
    ep: &Endpoint,
    commands: &[String],
    limit: usize,
    timeout: Duration,
) -> Result<Vec<Reply>> {
    let (mut writer, mut reader) = ep.open(timeout)?;
    let deadline = Instant::now() + timeout;
    reader.budget = limit;
    reader.deadline = Some(deadline);
    let timeout_handle = writer.try_clone()?;
    reader.set_timeout = Some(Box::new(move |t| timeout_handle.set_read_timeout(t)));

    // Written from a thread of its own: tor answers as it reads, and with
    // thousands of commands both socket buffers can fill, each side waiting on
    // the other. Reading while writing cannot deadlock.
    let mut payload = String::new();
    for c in commands {
        if c.contains(['\r', '\n']) {
            return Err(Error::Protocol(
                "a command may not contain a line break".into(),
            ));
        }
        payload.push_str(c);
        payload.push_str("\r\n");
    }
    payload.push_str("QUIT\r\n");
    let w = thread::spawn(move || {
        let r = writer.write_all(payload.as_bytes());
        let _ = writer.shutdown(Shutdown::Write);
        r
    });

    let mut replies = Vec::with_capacity(commands.len());
    while replies.len() < commands.len() {
        match reader.reply()? {
            Some(r) => replies.push(r),
            None => return Err(Error::Protocol("tor closed the session early".into())),
        }
    }
    let _ = w.join();
    Ok(replies)
}

/// `query` for one command, failing on anything but a 2xx.
pub fn query_one(ep: &Endpoint, command: &str, limit: usize, timeout: Duration) -> Result<Reply> {
    let mut r = query(ep, &[command.to_string()], limit, timeout)?;
    let reply = r.pop().ok_or_else(|| Error::Protocol("no reply".into()))?;
    if !reply.is_ok() {
        return Err(Error::Protocol(format!(
            "{} {}",
            reply.code,
            reply.first_text()
        )));
    }
    Ok(reply)
}

/// GETINFO for several keys in one command; values by key.
pub fn getinfo(
    ep: &Endpoint,
    keys: &[&str],
    limit: usize,
    timeout: Duration,
) -> Result<HashMap<String, String>> {
    let reply = query_one(ep, &format!("GETINFO {}", keys.join(" ")), limit, timeout)?;
    Ok(reply.values())
}

/// A long-lived session. Replies come back in order; events (650) go to the
/// receiver handed out by [`Conn::open`].
pub struct Conn {
    writer: TcpStream,
    replies: Receiver<Result<Reply>>,
}

impl Conn {
    /// Opens, authenticates and subscribes to `events`.
    pub fn open(ep: &Endpoint, events: &[&str]) -> Result<(Conn, Receiver<Reply>)> {
        let (writer, mut reader) = ep.open(Duration::from_secs(5))?;
        // No deadline on a session that is meant to stay open; the byte budget
        // is per message instead, reset as each one arrives.
        writer.set_read_timeout(None)?;
        let (rtx, rrx): (Sender<Result<Reply>>, _) = mpsc::channel();
        let (etx, erx) = mpsc::channel();
        thread::spawn(move || loop {
            reader.budget = 4 * 1024 * 1024;
            match reader.reply() {
                Ok(Some(r)) if r.code == 650 => {
                    if etx.send(r).is_err() {
                        break;
                    }
                }
                Ok(Some(r)) => {
                    if rtx.send(Ok(r)).is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    let _ = rtx.send(Err(e));
                    break;
                }
            }
        });
        let mut conn = Conn {
            writer,
            replies: rrx,
        };
        if !events.is_empty() {
            let r = conn.command(&format!("SETEVENTS {}", events.join(" ")))?;
            if !r.is_ok() {
                return Err(Error::Protocol(format!(
                    "SETEVENTS refused: {}",
                    r.first_text()
                )));
            }
        }
        Ok((conn, erx))
    }

    pub fn command(&mut self, line: &str) -> Result<Reply> {
        if line.contains(['\r', '\n']) {
            return Err(Error::Protocol(
                "a command may not contain a line break".into(),
            ));
        }
        self.writer.write_all(format!("{line}\r\n").as_bytes())?;
        match self.replies.recv_timeout(Duration::from_secs(10)) {
            Ok(r) => r,
            Err(RecvTimeoutError::Timeout) => Err(Error::Timeout),
            Err(RecvTimeoutError::Disconnected) => Err(Error::Unreachable("session closed".into())),
        }
    }

    pub fn getinfo(&mut self, keys: &[&str]) -> Result<HashMap<String, String>> {
        let r = self.command(&format!("GETINFO {}", keys.join(" ")))?;
        if !r.is_ok() {
            return Err(Error::Protocol(format!("{} {}", r.code, r.first_text())));
        }
        Ok(r.values())
    }

    pub fn close(&mut self) {
        let _ = self.writer.write_all(b"QUIT\r\n");
        let _ = self.writer.shutdown(Shutdown::Both);
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Vec<Reply> {
        let mut r = Reader::new(s.as_bytes(), usize::MAX);
        let mut out = Vec::new();
        while let Some(x) = r.reply().unwrap() {
            out.push(x);
        }
        out
    }

    #[test]
    fn single_and_multi_line_replies() {
        let r = parse("250-traffic/read=12\r\n250-traffic/written=34\r\n250 OK\r\n");
        assert_eq!(r.len(), 1);
        let v = r[0].values();
        assert_eq!(v["traffic/read"], "12");
        assert_eq!(v["traffic/written"], "34");
    }

    #[test]
    fn data_blocks_are_unstuffed_and_keyed() {
        let r = parse("250+ns/all=\r\nr a\r\n..dot\r\n.\r\n250 OK\r\n250 closing connection\r\n");
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].values()["ns/all"], "r a\n.dot\n");
        assert_eq!(r[1].first_text(), "closing connection");
    }

    #[test]
    fn a_cut_off_reply_is_an_error_not_a_shorter_answer() {
        let mut r = Reader::new("250+ns/all=\r\nr a\r\n".as_bytes(), usize::MAX);
        assert!(r.reply().is_err());
        let mut r = Reader::new("250-a=1\r\n".as_bytes(), usize::MAX);
        assert!(r.reply().is_err());
    }

    #[test]
    fn the_byte_budget_is_enforced_while_reading() {
        let big = format!("250+x=\r\n{}\r\n.\r\n250 OK\r\n", "a".repeat(1000));
        let mut r = Reader::new(big.as_bytes(), 500);
        assert!(matches!(r.reply(), Err(Error::TooLarge)));
    }

    #[test]
    fn garbage_is_a_protocol_error() {
        let mut r = Reader::new("hello\r\n".as_bytes(), usize::MAX);
        assert!(r.reply().is_err());
        let mut r = Reader::new("250?x\r\n".as_bytes(), usize::MAX);
        assert!(r.reply().is_err());
    }
}
