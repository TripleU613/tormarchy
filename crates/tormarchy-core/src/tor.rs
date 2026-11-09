//! What tor knows, asked and parsed: bootstrap progress, traffic, the circuit
//! carrying new streams and the relays on it, and which countries have exits.
//!
//! The parsers are pure functions over reply text, so each is tested without a
//! tor. The fetchers work over [`Ask`], which both the one-shot client and the
//! daemon's long-lived session implement.

use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

use serde::Serialize;

use crate::control::{self, Conn, Endpoint, Reply};

/// Anything that can put commands to tor and get their replies, in order.
pub trait Ask {
    fn ask(&mut self, commands: &[String], limit: usize) -> control::Result<Vec<Reply>>;
}

/// One session per call.
pub struct OneShot<'a> {
    pub endpoint: &'a Endpoint,
    pub timeout: Duration,
}

impl Ask for OneShot<'_> {
    fn ask(&mut self, commands: &[String], limit: usize) -> control::Result<Vec<Reply>> {
        control::query(self.endpoint, commands, limit, self.timeout)
    }
}

impl Ask for Conn {
    fn ask(&mut self, commands: &[String], _limit: usize) -> control::Result<Vec<Reply>> {
        commands.iter().map(|c| self.command(c)).collect()
    }
}

fn getinfo(ask: &mut dyn Ask, keys: &[&str]) -> control::Result<HashMap<String, String>> {
    let replies = ask.ask(
        &[format!("GETINFO {}", keys.join(" "))],
        control::REPLY_LIMIT,
    )?;
    let reply = replies
        .into_iter()
        .next()
        .ok_or_else(|| control::Error::Protocol("no reply".into()))?;
    if !reply.is_ok() {
        return Err(control::Error::Protocol(format!(
            "{} {}",
            reply.code,
            reply.first_text()
        )));
    }
    Ok(reply.values())
}

/// `PROGRESS=NN` from a bootstrap status line, clamped to 0..=100.
pub fn parse_bootstrap(phase: &str) -> u8 {
    phase
        .split_whitespace()
        .find_map(|w| w.strip_prefix("PROGRESS="))
        .and_then(|n| n.parse::<u32>().ok())
        .map(|n| n.min(100) as u8)
        .unwrap_or(0)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vitals {
    pub bootstrap: u8,
    pub bytes_read: u64,
    pub bytes_written: u64,
}

/// Bootstrap percentage and the session traffic counters, in one command.
pub fn vitals(ask: &mut dyn Ask) -> control::Result<Vitals> {
    let v = getinfo(
        ask,
        &["status/bootstrap-phase", "traffic/read", "traffic/written"],
    )?;
    let num = |k: &str| v.get(k).and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    Ok(Vitals {
        bootstrap: v
            .get("status/bootstrap-phase")
            .map(|s| parse_bootstrap(s))
            .unwrap_or(0),
        bytes_read: num("traffic/read"),
        bytes_written: num("traffic/written"),
    })
}

pub fn circuit_established(ask: &mut dyn Ask) -> bool {
    getinfo(ask, &["status/circuit-established"])
        .map(|v| {
            v.get("status/circuit-established")
                .map(|s| s.trim() == "1")
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

/// A built circuit as circuit-status lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Circuit {
    pub id: String,
    pub created: String,
    pub fingerprints: Vec<String>,
}

/// The circuit new streams ride on: the newest BUILT general-purpose one.
///
/// PURPOSE=GENERAL is not enough on its own. Tor also builds one-hop internal
/// circuits for its own directory fetches and tags those GENERAL too, so a
/// naive "last GENERAL circuit" picks a single-hop tunnel to a directory cache
/// and presents it as where traffic goes. Newest by TIME_CREATED, not by
/// position: circuit-status promises no order. The timestamp is fixed-width ISO
/// 8601 in UTC, so it orders as a string.
pub fn pick_circuit(circuit_status: &str) -> Option<Circuit> {
    let mut best: Option<Circuit> = None;
    for line in circuit_status.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 3 || fields[1] != "BUILT" {
            continue;
        }
        if !fields.contains(&"PURPOSE=GENERAL")
            || fields
                .iter()
                .any(|f| f.contains("ONEHOP_TUNNEL") || f.contains("IS_INTERNAL"))
        {
            continue;
        }
        let created = fields
            .iter()
            .find_map(|f| f.strip_prefix("TIME_CREATED="))
            .unwrap_or("")
            .to_string();
        let fingerprints: Vec<String> = fields[2]
            .split(',')
            .map(|hop| {
                hop.split(['~', '='])
                    .next()
                    .unwrap_or("")
                    .trim_start_matches('$')
                    .to_string()
            })
            .filter(|fp| !fp.is_empty())
            .collect();
        if fingerprints.is_empty() {
            continue;
        }
        let newer = match &best {
            None => true,
            Some(b) => created.as_str() >= b.created.as_str(),
        };
        if newer {
            best = Some(Circuit {
                id: fields[0].to_string(),
                created,
                fingerprints,
            });
        }
    }
    best
}

/// What the consensus says about one relay.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Relay {
    pub nickname: String,
    pub ip: String,
    /// Only the ones worth showing.
    pub flags: Vec<String>,
    /// The consensus weight's figure, in kilobytes per second.
    pub bandwidth_kbps: u64,
}

const SHOWN_FLAGS: [&str; 4] = ["Guard", "Exit", "Fast", "Stable"];

/// Every relay in a stretch of network-status text, in order. The `s` and `w`
/// lines after each `r` belong to that relay.
pub fn parse_relays(ns: &str) -> Vec<Relay> {
    let mut out: Vec<Relay> = Vec::new();
    for line in ns.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("r ") {
            let f: Vec<&str> = rest.split_whitespace().collect();
            // r nickname identity digest date time IP ORPort DirPort
            if f.len() >= 6 {
                let nick = f[0];
                out.push(Relay {
                    // Nicknames are [A-Za-z0-9]{1,19} by the directory spec;
                    // anything else is not trusted onto the panel.
                    nickname: if nick.len() <= 19 && nick.chars().all(|c| c.is_ascii_alphanumeric())
                    {
                        nick.to_string()
                    } else {
                        String::new()
                    },
                    ip: f[5].to_string(),
                    ..Default::default()
                });
            }
        } else if let Some(rest) = line.strip_prefix("s ") {
            if let Some(r) = out.last_mut() {
                r.flags = rest
                    .split_whitespace()
                    .filter(|f| SHOWN_FLAGS.contains(f))
                    .map(str::to_string)
                    .collect();
            }
        } else if let Some(rest) = line.strip_prefix("w ") {
            if let Some(r) = out.last_mut() {
                r.bandwidth_kbps = rest
                    .split_whitespace()
                    .find_map(|w| w.strip_prefix("Bandwidth="))
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0);
            }
        }
    }
    out
}

/// The address of every usable exit in a consensus, each once.
pub fn exit_addresses(ns: &str) -> Vec<String> {
    let mut ips = BTreeSet::new();
    let mut ip: Option<String> = None;
    for line in ns.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("r ") {
            ip = rest.split_whitespace().nth(5).map(str::to_string);
        } else if let Some(rest) = line.strip_prefix("s ") {
            let flags: Vec<&str> = rest.split_whitespace().collect();
            if flags.contains(&"Exit") && !flags.contains(&"BadExit") {
                if let Some(a) = ip.take() {
                    ips.insert(a);
                }
            }
            ip = None;
        }
    }
    ips.into_iter().collect()
}

/// A two-letter country code from an ip-to-country answer, or "xx".
pub fn country_of(answer: &str) -> String {
    let cc = answer.trim().to_ascii_lowercase();
    if cc.len() == 2 && cc.chars().all(|c| c.is_ascii_lowercase()) && cc != "??" {
        cc
    } else {
        "xx".into()
    }
}

/// One hop of the circuit, ready to show.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Hop {
    pub cc: String,
    pub nickname: String,
    pub flags: Vec<String>,
    #[serde(rename = "bandwidthKBps")]
    pub bandwidth_kbps: u64,
    #[serde(skip)]
    pub ip: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    pub circuit_id: String,
    pub created: String,
    pub hops: Vec<Hop>,
}

impl Path {
    pub fn exit_ip(&self) -> &str {
        self.hops.last().map(|h| h.ip.as_str()).unwrap_or("")
    }
    pub fn exit_country(&self) -> &str {
        match self.hops.last().map(|h| h.cc.as_str()) {
            Some("xx") | None => "",
            Some(cc) => cc,
        }
    }
    pub fn age_seconds(&self, now: u64) -> u64 {
        crate::sys::parse_utc(&self.created)
            .map(|t| (now as i64 - t).max(0) as u64)
            .unwrap_or(0)
    }
}

/// The circuit carrying new streams, every hop resolved.
///
/// Every hop must resolve or nothing is reported. A partial answer drawn as a
/// complete path is worse than none: two hops shown as the whole circuit reads
/// as a shorter, weaker path than the one in use.
pub fn current_path(ask: &mut dyn Ask) -> control::Result<Option<Path>> {
    let status = getinfo(ask, &["circuit-status"])?;
    let Some(circuit) = status.get("circuit-status").and_then(|s| pick_circuit(s)) else {
        return Ok(None);
    };
    resolve_path(ask, circuit)
}

pub fn resolve_path(ask: &mut dyn Ask, circuit: Circuit) -> control::Result<Option<Path>> {
    // One reply per fingerprint, each carrying a single relay, in request order.
    let commands: Vec<String> = circuit
        .fingerprints
        .iter()
        .map(|fp| format!("GETINFO ns/id/${fp}"))
        .collect();
    let replies = ask.ask(&commands, control::REPLY_LIMIT)?;
    let mut relays = Vec::new();
    for reply in &replies {
        if !reply.is_ok() {
            return Ok(None);
        }
        let ns: String = reply.lines.iter().filter_map(|l| l.data.clone()).collect();
        match parse_relays(&ns).into_iter().next() {
            Some(r) => relays.push(r),
            None => return Ok(None),
        }
    }
    if relays.len() != circuit.fingerprints.len() {
        return Ok(None);
    }
    // Matched by the address in the key, not trusted to arrive in order.
    let keys: Vec<String> = relays
        .iter()
        .map(|r| format!("ip-to-country/{}", r.ip))
        .collect();
    let key_refs: Vec<&str> = keys.iter().map(String::as_str).collect();
    let geo = getinfo(ask, &key_refs).unwrap_or_default();
    let hops = relays
        .into_iter()
        .map(|r| Hop {
            cc: country_of(
                geo.get(&format!("ip-to-country/{}", r.ip))
                    .map(String::as_str)
                    .unwrap_or(""),
            ),
            nickname: r.nickname,
            flags: r.flags,
            bandwidth_kbps: r.bandwidth_kbps,
            ip: r.ip,
        })
        .collect();
    Ok(Some(Path {
        circuit_id: circuit.id,
        created: circuit.created,
        hops,
    }))
}

/// Every country with a usable exit right now, as sorted two-letter codes.
///
/// The consensus is the one large reply: it has its own ceiling, and must end
/// with its terminator. One cut off at a limit has no end, and parsed anyway it
/// was a list missing every country past the cut, given as the whole list.
pub fn exit_countries(endpoint: &Endpoint) -> Result<Vec<String>, String> {
    let replies = control::query(
        endpoint,
        &["GETINFO ns/all".to_string()],
        control::CONSENSUS_LIMIT,
        Duration::from_secs(60),
    )
    .map_err(|e| match e {
        control::Error::TooLarge | control::Error::Timeout | control::Error::Protocol(_) => {
            "tor's relay list came back incomplete; try again in a moment".to_string()
        }
        other => other.to_string(),
    })?;
    let reply = replies.into_iter().next().ok_or("no reply")?;
    if !reply.is_ok() {
        return Err(format!("tor refused: {}", reply.first_text()));
    }
    let ns = reply
        .values()
        .remove("ns/all")
        .ok_or("tor's relay list came back incomplete; try again in a moment")?;
    let ips = exit_addresses(&ns);
    if ips.is_empty() {
        return Err("no exit relays in the consensus yet".into());
    }
    let mut countries = BTreeSet::new();
    // Chunks keep each session's reply small and its deadline meaningful.
    for chunk in ips.chunks(200) {
        let keys: Vec<String> = chunk
            .iter()
            .map(|ip| format!("ip-to-country/{ip}"))
            .collect();
        let command = format!("GETINFO {}", keys.join(" "));
        let replies = control::query(
            endpoint,
            &[command],
            control::REPLY_LIMIT,
            Duration::from_secs(30),
        )
        .map_err(|e| e.to_string())?;
        for reply in replies {
            for value in reply.values().into_values() {
                let cc = country_of(&value);
                if cc != "xx" {
                    countries.insert(cc);
                }
            }
        }
    }
    Ok(countries.into_iter().collect())
}

/// Circuits that carry exit traffic. Since conflux (tor 0.4.8) most streams
/// ride CONFLUX_LINKED legs, not GENERAL ones, so both count. Tor's own one-hop
/// and internal circuits carry no user streams and are left alone.
fn exit_circuits(circuit_status: &str) -> Vec<&str> {
    circuit_status
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            let carries = f.len() >= 3
                && f[1] == "BUILT"
                && f.iter().any(|x| {
                    matches!(
                        *x,
                        "PURPOSE=GENERAL" | "PURPOSE=CONFLUX_LINKED" | "PURPOSE=CONFLUX_UNLINKED"
                    )
                })
                && !f
                    .iter()
                    .any(|x| x.contains("ONEHOP_TUNNEL") || x.contains("IS_INTERNAL"));
            carries.then(|| f[0])
        })
        .collect()
}

/// NEWNYM only steers new streams. A browser keeps its HTTP/2 connections open
/// and reuses them, so a site already loaded goes on seeing the old exit -- the
/// old country, even after an exit pin -- until the connection idles out.
/// Closing the circuits drops those streams and the browser reconnects through
/// a fresh one. Best effort: a circuit can vanish between list and close.
fn close_exit_circuits(ask: &mut dyn Ask) {
    let Ok(status) = getinfo(ask, &["circuit-status"]) else {
        return;
    };
    let Some(status) = status.get("circuit-status") else {
        return;
    };
    let close: Vec<String> = exit_circuits(status)
        .into_iter()
        .map(|id| format!("CLOSECIRCUIT {id}"))
        .collect();
    if !close.is_empty() {
        let _ = ask.ask(&close, control::REPLY_LIMIT);
    }
}

pub fn newnym(ask: &mut dyn Ask) -> Result<(), String> {
    let replies = ask
        .ask(&["SIGNAL NEWNYM".to_string()], control::REPLY_LIMIT)
        .map_err(|e| e.to_string())?;
    match replies.first() {
        Some(r) if r.is_ok() => {
            close_exit_circuits(ask);
            Ok(())
        }
        Some(r) => Err(format!(
            "tor refused the NEWNYM request: {} {}",
            r.code,
            r.first_text()
        )),
        None => Err("no reply to NEWNYM".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_progress() {
        assert_eq!(
            parse_bootstrap("NOTICE BOOTSTRAP PROGRESS=100 TAG=done SUMMARY=\"Done\""),
            100
        );
        assert_eq!(parse_bootstrap("NOTICE BOOTSTRAP PROGRESS=45 TAG=x"), 45);
        assert_eq!(parse_bootstrap("PROGRESS=999"), 100);
        assert_eq!(parse_bootstrap("nothing"), 0);
    }

    const CIRCUITS: &str = "\
5 BUILT $AAA~a,$BBB~b,$CCC~c BUILD_FLAGS=NEED_CAPACITY PURPOSE=GENERAL TIME_CREATED=2026-11-09T10:00:00.000000
6 BUILT $DDD~d BUILD_FLAGS=ONEHOP_TUNNEL,IS_INTERNAL PURPOSE=GENERAL TIME_CREATED=2026-11-09T12:00:00.000000
7 BUILT $EEE~e,$FFF~f,$GGG~g PURPOSE=GENERAL TIME_CREATED=2026-11-09T11:00:00.000000
8 EXTENDED $HHH~h PURPOSE=GENERAL TIME_CREATED=2026-11-09T13:00:00.000000
9 BUILT $III~i,$JJJ~j,$KKK~k PURPOSE=HS_CLIENT_REND TIME_CREATED=2026-11-09T14:00:00.000000
";

    #[test]
    fn newnym_closes_every_circuit_a_stream_can_ride() {
        let status = format!(
            "{CIRCUITS}30 BUILT $LLL~l,$MMM~m,$NNN~n BUILD_FLAGS=NEED_CAPACITY,NEED_UPTIME PURPOSE=CONFLUX_LINKED TIME_CREATED=2026-11-09T09:00:00.000000\n"
        );
        assert_eq!(
            exit_circuits(&status),
            ["5", "7", "30"],
            "general and conflux legs close; one-hop, unbuilt and onion circuits stay"
        );
    }

    #[test]
    fn the_newest_general_three_hop_circuit_is_the_one() {
        let c = pick_circuit(CIRCUITS).unwrap();
        assert_eq!(
            c.id, "7",
            "newest by time, not position; one-hop internal tunnels and other purposes skipped"
        );
        assert_eq!(c.fingerprints, ["EEE", "FFF", "GGG"]);
        assert_eq!(pick_circuit("8 LAUNCHED  PURPOSE=GENERAL"), None);
    }

    const NS: &str = "\
r guardy AAAA BBBB 2026-11-09 00:00:00 192.0.2.1 9001 0
s Fast Guard Running Stable Valid
w Bandwidth=12000
r bad$nick AAAA BBBB 2026-11-09 00:00:00 192.0.2.2 9001 0
s Exit BadExit Fast Running Valid
w Bandwidth=5
r exity AAAA BBBB 2026-11-09 00:00:00 192.0.2.3 9001 0
s Exit Fast Running Valid
";

    #[test]
    fn relays_carry_their_flags_and_bandwidth() {
        let r = parse_relays(NS);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].nickname, "guardy");
        assert_eq!(r[0].flags, ["Fast", "Guard", "Stable"]);
        assert_eq!(r[0].bandwidth_kbps, 12000);
        assert_eq!(
            r[1].nickname, "",
            "a nickname outside the spec is not shown"
        );
        assert_eq!(r[2].bandwidth_kbps, 0);
    }

    #[test]
    fn exits_exclude_bad_exits() {
        assert_eq!(exit_addresses(NS), ["192.0.2.3"]);
    }

    #[test]
    fn countries() {
        assert_eq!(country_of("DE"), "de");
        assert_eq!(country_of("??"), "xx");
        assert_eq!(country_of(""), "xx");
        assert_eq!(country_of("deu"), "xx");
    }

    struct Fake(Vec<(String, String)>);
    impl Ask for Fake {
        fn ask(&mut self, commands: &[String], _: usize) -> control::Result<Vec<Reply>> {
            Ok(commands
                .iter()
                .map(|c| {
                    let text = self
                        .0
                        .iter()
                        .find(|(k, _)| c.starts_with(k.as_str()))
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default();
                    let mut r = control::Reader::new(text.as_bytes(), usize::MAX);
                    r.reply().unwrap().unwrap_or(Reply {
                        code: 552,
                        lines: vec![],
                    })
                })
                .collect())
        }
    }

    #[test]
    fn a_path_resolves_every_hop_or_nothing() {
        let ns = |nick: &str, ip: &str| {
            format!("250+ns/id/x=\r\nr {nick} A B 2026-11-09 00:00:00 {ip} 9001 0\r\ns Fast Running\r\n.\r\n250 OK\r\n")
        };
        let mut fake = Fake(vec![
            ("GETINFO circuit-status".into(), format!("250+circuit-status=\r\n{}.\r\n250 OK\r\n", CIRCUITS.replace('\n', "\r\n"))),
            ("GETINFO ns/id/$EEE".into(), ns("one", "192.0.2.1")),
            ("GETINFO ns/id/$FFF".into(), ns("two", "192.0.2.2")),
            ("GETINFO ns/id/$GGG".into(), ns("three", "192.0.2.3")),
            (
                "GETINFO ip-to-country".into(),
                "250-ip-to-country/192.0.2.1=se\r\n250-ip-to-country/192.0.2.2=ch\r\n250-ip-to-country/192.0.2.3=??\r\n250 OK\r\n".into(),
            ),
        ]);
        let p = current_path(&mut fake).unwrap().unwrap();
        let ccs: Vec<&str> = p.hops.iter().map(|h| h.cc.as_str()).collect();
        assert_eq!(ccs, ["se", "ch", "xx"]);
        assert_eq!(p.exit_ip(), "192.0.2.3");
        assert_eq!(
            p.exit_country(),
            "",
            "an unplaced exit is no country, not \"xx\""
        );

        let mut missing = Fake(vec![
            (
                "GETINFO circuit-status".into(),
                format!(
                    "250+circuit-status=\r\n{}.\r\n250 OK\r\n",
                    CIRCUITS.replace('\n', "\r\n")
                ),
            ),
            ("GETINFO ns/id/$EEE".into(), ns("one", "192.0.2.1")),
        ]);
        assert_eq!(
            current_path(&mut missing).unwrap(),
            None,
            "two of three hops is no path"
        );
    }
}

#[cfg(test)]
mod socket_tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::thread;

    /// A control port on a real socket: answers AUTHENTICATE, a full-size
    /// consensus (finished or cut off), and ip-to-country lookups.
    fn fake_tor(relays: usize, finish: bool) -> (Endpoint, std::path::PathBuf) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        thread::spawn(move || {
            for conn in l.incoming() {
                let Ok(mut s) = conn else { break };
                let reader = BufReader::new(s.try_clone().unwrap());
                let mut out = Vec::new();
                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    if line.starts_with("QUIT") {
                        break;
                    } else if line.starts_with("AUTHENTICATE") {
                        s.write_all(b"250 OK\r\n").unwrap();
                    } else if line.starts_with("GETINFO ns/all") {
                        out.extend_from_slice(b"250+ns/all=\r\n");
                        for i in 0..relays {
                            let ip = if i == relays - 1 {
                                "10.9.9.9"
                            } else {
                                "10.0.0.1"
                            };
                            out.extend_from_slice(
                                format!("r relay{i} AAAA BBBB 2026-11-09 00:00:00 {ip} 9001 0\r\ns Exit Fast Guard HSDir Running Stable V2Dir Valid\r\nw Bandwidth=1000\r\n").as_bytes(),
                            );
                        }
                        if finish {
                            out.extend_from_slice(b".\r\n250 OK\r\n");
                        }
                    } else if let Some(keys) = line.strip_prefix("GETINFO ") {
                        for k in keys.split_whitespace() {
                            let ip = k.trim_start_matches("ip-to-country/");
                            let cc = if ip == "10.9.9.9" { "zz" } else { "de" };
                            out.extend_from_slice(format!("250-{k}={cc}\r\n").as_bytes());
                        }
                        out.extend_from_slice(b"250 OK\r\n");
                    }
                }
                if finish {
                    out.extend_from_slice(b"250 closing connection\r\n");
                }
                let _ = s.write_all(&out);
            }
        });
        let dir = crate::sys::create_temp_dir("tm-ctl-").unwrap();
        std::fs::write(dir.join("cookie"), [7u8; 32]).unwrap();
        (
            Endpoint {
                port,
                cookies: vec![dir.join("cookie")],
            },
            dir,
        )
    }

    #[test]
    fn a_full_size_consensus_is_read_to_its_end() {
        let (ep, dir) = fake_tor(8000, true);
        assert_eq!(
            exit_countries(&ep).unwrap(),
            ["de", "zz"],
            "the last relay's country is listed"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_consensus_cut_off_before_its_end_is_refused() {
        let (ep, dir) = fake_tor(8000, false);
        let err = exit_countries(&ep).unwrap_err();
        assert!(err.contains("incomplete"), "{err}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_poll_limit_still_applies_to_everything_else() {
        let (ep, dir) = fake_tor(8000, true);
        let r = control::query(
            &ep,
            &["GETINFO ns/all".into()],
            control::REPLY_LIMIT,
            Duration::from_secs(10),
        );
        assert!(matches!(r, Err(control::Error::TooLarge)));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
