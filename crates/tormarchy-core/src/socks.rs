//! Round trips through tor's SOCKS port, timed.
//!
//! A Tor circuit carries only TCP, so there is no ping through one. What there is
//! is a request: the first pays for the handshake and the three-hop setup, and
//! every one after on the same connection is a single round trip. So a live
//! reading holds one connection open and re-times a HEAD on it. A HEAD carries
//! no content, so the exit learns only that someone asked for a front page --
//! which it learns from any Tor user.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

/// Hosts a route is tried against, a different one each attempt, so one site
/// being down is not "no route".
pub const PROBE_URLS: [&str; 3] = [
    "http://check.torproject.org/",
    "http://www.torproject.org/",
    "http://one.one.one.one/",
];
pub const LATENCY_URL: &str = "http://www.torproject.org/";

/// The most a response header may be before it is not one.
const HEADER_LIMIT: usize = 16 * 1024;

pub struct Stream {
    sock: TcpStream,
    host: String,
}

fn split_url(url: &str) -> io::Result<(String, u16)> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "only plain http is timed"))?;
    let authority = rest.split('/').next().unwrap_or("");
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse()
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "bad port"))?,
        ),
        None => (authority.to_string(), 80),
    };
    if host.is_empty()
        || host.len() > 255
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "bad host"));
    }
    Ok((host, port))
}

impl Stream {
    /// A connection to `url`'s host through the SOCKS port, the name resolved
    /// by tor, never locally.
    pub fn open(socks_port: u16, url: &str, timeout: Duration) -> io::Result<Stream> {
        let (host, port) = split_url(url)?;
        let deadline = Instant::now() + timeout;
        let mut sock =
            TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], socks_port)), timeout)?;
        sock.set_nodelay(true)?;
        let left = |d: Instant| {
            d.checked_duration_since(Instant::now())
                .unwrap_or(Duration::from_millis(1))
                .max(Duration::from_millis(1))
        };
        sock.set_write_timeout(Some(left(deadline)))?;
        sock.set_read_timeout(Some(left(deadline)))?;

        sock.write_all(&[5, 1, 0])?;
        let mut hello = [0u8; 2];
        sock.read_exact(&mut hello)?;
        if hello != [5, 0] {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                "the SOCKS proxy refused",
            ));
        }
        let mut req = vec![5, 1, 0, 3, host.len() as u8];
        req.extend_from_slice(host.as_bytes());
        req.extend_from_slice(&port.to_be_bytes());
        sock.write_all(&req)?;
        sock.set_read_timeout(Some(left(deadline)))?;
        let mut head = [0u8; 4];
        sock.read_exact(&mut head)?;
        if head[0] != 5 || head[1] != 0 {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                format!("tor could not connect (SOCKS reply {})", head[1]),
            ));
        }
        let skip = match head[3] {
            1 => 4 + 2,
            4 => 16 + 2,
            3 => {
                let mut n = [0u8; 1];
                sock.read_exact(&mut n)?;
                n[0] as usize + 2
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "bad SOCKS reply",
                ))
            }
        };
        let mut rest = vec![0u8; skip];
        sock.read_exact(&mut rest)?;
        Ok(Stream { sock, host })
    }

    /// One HEAD on this connection: milliseconds to the first byte of the
    /// answer, with the whole header read so the connection is ready for the
    /// next. Any status counts -- a 301 still proves the round trip.
    pub fn time_head(&mut self, timeout: Duration) -> io::Result<u64> {
        let request = format!(
            "HEAD / HTTP/1.1\r\nHost: {}\r\nUser-Agent: Mozilla/5.0\r\nAccept: */*\r\nConnection: keep-alive\r\n\r\n",
            self.host
        );
        let deadline = Instant::now() + timeout;
        self.sock.set_write_timeout(Some(timeout))?;
        let start = Instant::now();
        self.sock.write_all(request.as_bytes())?;
        let mut header = Vec::new();
        let mut first: Option<Duration> = None;
        let mut buf = [0u8; 2048];
        loop {
            let left = deadline
                .checked_duration_since(Instant::now())
                .ok_or(io::ErrorKind::TimedOut)?;
            self.sock
                .set_read_timeout(Some(left.max(Duration::from_millis(1))))?;
            let n = self.sock.read(&mut buf)?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the connection closed",
                ));
            }
            first.get_or_insert_with(|| start.elapsed());
            header.extend_from_slice(&buf[..n]);
            if header.len() > HEADER_LIMIT {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "an endless response header",
                ));
            }
            if let Some(end) = find(&header, b"\r\n\r\n") {
                if !header.starts_with(b"HTTP/") {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "not an HTTP answer",
                    ));
                }
                // A HEAD answer has no body; anything after the header is not
                // ours to keep, and a server that sends one closes after.
                let _ = end;
                break;
            }
        }
        let ms = first.unwrap_or_default().as_millis() as u64;
        Ok(ms.max(1))
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// One request on a fresh connection; the round trip, handshake included.
pub fn time_request(socks_port: u16, url: &str, timeout: Duration) -> io::Result<u64> {
    let start = Instant::now();
    let mut s = Stream::open(socks_port, url, timeout)?;
    let left = timeout
        .checked_sub(start.elapsed())
        .unwrap_or(Duration::from_millis(1));
    s.time_head(left)?;
    Ok(start.elapsed().as_millis().max(1) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    /// A SOCKS5 proxy and web server in one: answers the handshake, then a HEAD
    /// per request after `delay`.
    fn fake(delay: Duration, requests: usize) -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut b = [0u8; 512];
            s.read_exact(&mut b[..3]).unwrap();
            s.write_all(&[5, 0]).unwrap();
            s.read_exact(&mut b[..5]).unwrap();
            let n = b[4] as usize;
            s.read_exact(&mut b[..n + 2]).unwrap();
            s.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
            for _ in 0..requests {
                let mut got = Vec::new();
                while find(&got, b"\r\n\r\n").is_none() {
                    let k = s.read(&mut b).unwrap();
                    if k == 0 {
                        return;
                    }
                    got.extend_from_slice(&b[..k]);
                }
                thread::sleep(delay);
                s.write_all(
                    b"HTTP/1.1 301 Moved\r\nLocation: https://x/\r\nContent-Length: 0\r\n\r\n",
                )
                .unwrap();
            }
        });
        port
    }

    #[test]
    fn a_kept_connection_is_retimed() {
        let port = fake(Duration::from_millis(60), 3);
        let mut s = Stream::open(port, LATENCY_URL, Duration::from_secs(5)).unwrap();
        for _ in 0..3 {
            let ms = s.time_head(Duration::from_secs(5)).unwrap();
            assert!((50..1000).contains(&ms), "{ms}");
        }
    }

    #[test]
    fn https_and_odd_hosts_are_refused() {
        assert!(split_url("https://x/").is_err());
        assert!(split_url("http://a b/").is_err());
        assert_eq!(
            split_url("http://host:8080/x").unwrap(),
            ("host".into(), 8080)
        );
    }

    #[test]
    fn nothing_listening_is_an_error_not_a_hang() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        assert!(time_request(port, LATENCY_URL, Duration::from_secs(2)).is_err());
    }
}
