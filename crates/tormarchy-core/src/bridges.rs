//! Bridges: lines as people paste them, validated, counted, and turned into a
//! torrc snippet tor has read before anything restarts.
//!
//! The lines are private -- which bridges you use is what a bridge exists to
//! hide -- so they arrive on stdin only, live in a root-only file, and nothing
//! about them leaves but transport names and counts.

use std::collections::BTreeMap;
use std::fs;

use serde::Serialize;

use crate::host::Host;
use crate::paths::Paths;
use crate::sys;
use crate::{Fail, Res};

/// More than any real set of bridge lines; past it, input is not bridge lines.
pub const INPUT_LIMIT: usize = 65536;

/// Normalised lines, or a refusal when there is more than [`INPUT_LIMIT`].
///
/// Refused whole rather than cut: cut at the limit, the last line ended
/// wherever the cut fell -- inside a cert, still shaped like a bridge line --
/// and went live with the rest dropped. Carriage returns go, and so does a
/// leading `Bridge `, torrc's own prefix, put back on activation. A NUL becomes
/// a control character the line check rejects, rather than vanishing.
pub fn normalize(raw: &[u8]) -> Result<String, Fail> {
    if raw.len() > INPUT_LIMIT {
        return Err(Fail(format!(
            "more than {} KiB of input; that is not a set of bridge lines",
            INPUT_LIMIT / 1024
        )));
    }
    let text: String = String::from_utf8_lossy(raw)
        .chars()
        .map(|c| if c == '\0' { '\u{1}' } else { c })
        .collect();
    let mut out = String::new();
    for line in text.split('\n') {
        let line = line.replace('\r', "");
        let mut l = line.trim_start();
        if let Some(rest) = l.strip_prefix("Bridge") {
            if rest.starts_with(char::is_whitespace) {
                l = rest.trim_start();
            }
        }
        out.push_str(l);
        out.push('\n');
    }
    // No trailing blank line beyond the input's own.
    while out.ends_with("\n\n") {
        out.pop();
    }
    if !text.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    Ok(out)
}

fn is_address(s: &str) -> bool {
    let Some((host, port)) = s.rsplit_once(':') else {
        return false;
    };
    if port.is_empty() || !port.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        return !inner.is_empty() && inner.chars().all(|c| c.is_ascii_hexdigit() || c == ':');
    }
    !host.is_empty() && host.chars().all(|c| c.is_ascii_digit() || c == '.')
}

fn is_transport(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// What one line is.
#[derive(Debug, PartialEq)]
pub enum Kind {
    Skip,
    Plain,
    Transport(String),
    Invalid,
}

/// A backslash or a control character makes any line invalid: each becomes a
/// torrc line, and a trailing backslash would carry it onto the next.
pub fn classify(line: &str) -> Kind {
    let t = line.trim_start();
    if t.is_empty() || t.starts_with('#') {
        return Kind::Skip;
    }
    if line.chars().any(|c| c == '\\' || c.is_control()) {
        return Kind::Invalid;
    }
    let mut f = t.split_whitespace();
    let first = f.next().unwrap_or("");
    if is_address(first) {
        return Kind::Plain;
    }
    if is_transport(first) && f.next().map(is_address).unwrap_or(false) {
        return Kind::Transport(first.to_string());
    }
    Kind::Invalid
}

/// Transport counts ("plain" for bridges without one) and invalid lines.
pub fn count(text: &str) -> (BTreeMap<String, usize>, usize) {
    let mut counts = BTreeMap::new();
    let mut invalid = 0;
    for line in text.lines() {
        match classify(line) {
            Kind::Skip => {}
            Kind::Plain => *counts.entry("plain".to_string()).or_insert(0) += 1,
            Kind::Transport(t) => *counts.entry(t).or_insert(0) += 1,
            Kind::Invalid => invalid += 1,
        }
    }
    (counts, invalid)
}

/// The transports a set of lines needs, plain bridges aside.
pub fn transports(text: &str) -> Vec<String> {
    count(text).0.into_keys().filter(|t| t != "plain").collect()
}

pub fn install_hint(transport: &str) -> String {
    match transport {
        "obfs4" | "obfs3" | "scramblesuit" => "omarchy-pkg-aur-add obfs4proxy".into(),
        "meek" | "meek_lite" => "omarchy-pkg-add meek".into(),
        "snowflake" => {
            "not packaged for Arch; build snowflake-client from the Tor anti-censorship repos"
                .into()
        }
        "webtunnel" => {
            "not packaged for Arch; build webtunnel-client from the Tor anti-censorship repos"
                .into()
        }
        other => format!("unknown transport '{other}'"),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct TransportInfo {
    pub name: String,
    pub count: usize,
    pub available: bool,
    #[serde(rename = "installHint")]
    pub install_hint: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Report {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(rename = "invalidLines", skip_serializing_if = "Option::is_none")]
    pub invalid_lines: Option<usize>,
    pub count: usize,
    pub transports: Vec<TransportInfo>,
}

pub fn report(host: &dyn Host, counts: &BTreeMap<String, usize>) -> Report {
    let mut r = Report::default();
    for (name, n) in counts {
        r.count += n;
        let available = name == "plain" || host.transport_binary(name).is_some();
        r.transports.push(TransportInfo {
            name: name.clone(),
            count: *n,
            available,
            install_hint: if available {
                String::new()
            } else {
                install_hint(name)
            },
        });
    }
    r
}

/// "active", "stashed" or "off".
pub fn state(paths: &Paths) -> &'static str {
    if paths.torrc_bridges.exists() {
        "active"
    } else if paths.bridge_summary.exists() || paths.bridge_stash.exists() {
        "stashed"
    } else {
        "off"
    }
}

/// Counts from the summary the panel can read, or the stash when root.
pub fn saved_counts(paths: &Paths) -> BTreeMap<String, usize> {
    if let Some(stash) = sys::read_bounded_string(&paths.bridge_stash, INPUT_LIMIT) {
        return count(&stash).0;
    }
    let mut out = BTreeMap::new();
    if let Some(summary) = sys::read_bounded_string(&paths.bridge_summary, 4096) {
        for line in summary.lines() {
            let mut f = line.split_whitespace();
            if let (Some(t), Some(n)) = (f.next(), f.next()) {
                if is_transport(t) {
                    if let Ok(n) = n.parse() {
                        out.insert(t.to_string(), n);
                    }
                }
            }
        }
    }
    out
}

/// Root. Refreshes the readable summary from the stash.
pub fn write_summary(paths: &Paths) -> Res {
    if !sys::is_root() {
        return Ok(());
    }
    let Some(stash) = sys::read_bounded_string(&paths.bridge_stash, INPUT_LIMIT) else {
        sys::remove_file_quiet(&paths.bridge_summary);
        return Ok(());
    };
    let text: String = count(&stash)
        .0
        .iter()
        .map(|(t, n)| format!("{t} {n}\n"))
        .collect();
    sys::write_atomic(&paths.bridge_summary, text.as_bytes(), 0o644)
        .map_err(|e| Fail(e.to_string()))
}

/// Turns bridge lines into the torrc snippet, refusing any transport this
/// machine cannot run, and keeping it only once tor's own parser has read it.
///
/// The shape check passes a truncated fingerprint or a port past 65535, and tor
/// refuses to start on either: the kill switch then holds a network nothing can
/// bring back. So the previous file is kept until tor has read the new one, and
/// put back if it will not.
pub fn activate(host: &dyn Host, lines: &str) -> Res {
    let paths = host.paths();
    let missing: Vec<String> = transports(lines)
        .into_iter()
        .filter(|t| host.transport_binary(t).is_none())
        .collect();
    if !missing.is_empty() {
        let mut msg = String::from("these bridge lines need transports that are not installed:");
        for t in &missing {
            msg.push_str(&format!("\n  {t:<12} install with: {}", install_hint(t)));
        }
        return Err(Fail(msg));
    }
    let (_, invalid) = count(lines);
    if invalid > 0 {
        return Err(Fail(format!(
            "{invalid} line(s) are not bridge lines; nothing was changed"
        )));
    }

    sys::ensure_dir(&paths.torrc_dir, 0o755).map_err(|e| Fail(e.to_string()))?;
    let previous = fs::read(&paths.torrc_bridges).ok();

    let mut conf =
        String::from("# Managed by tormarchy. Edit with 'tormarchy bridge set'.\nUseBridges 1\n");
    for t in transports(lines) {
        let bin = host.transport_binary(&t).expect("checked above");
        conf.push_str(&format!(
            "ClientTransportPlugin {t} exec {}\n",
            bin.display()
        ));
    }
    for line in lines.lines() {
        if matches!(classify(line), Kind::Plain | Kind::Transport(_)) {
            conf.push_str(&format!("Bridge {}\n", line.trim()));
        }
    }
    sys::write_atomic(&paths.torrc_bridges, conf.as_bytes(), 0o600)
        .map_err(|e| Fail(e.to_string()))?;

    if let Err(why) = host.tor_verify() {
        match previous {
            Some(old) => {
                let _ = sys::write_atomic(&paths.torrc_bridges, &old, 0o600);
            }
            None => sys::remove_file_quiet(&paths.torrc_bridges),
        }
        return Err(Fail(format!(
            "refusing to write a config tor cannot load: {why}"
        )));
    }
    Ok(())
}

/// Bridges change how tor reaches the network at all, which a reload does not
/// renegotiate.
pub fn reload(host: &dyn Host) -> Res {
    if !host.tor_running() {
        return Ok(());
    }
    host.systemctl(&["reset-failed", "tor.service"]);
    if !host.systemctl(&["restart", "tor.service"]) {
        return Err(Fail("wrote the config but could not restart tor".into()));
    }
    host.publish_cookie();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasted_lines_are_normalised() {
        let got =
            normalize(b"  Bridge obfs4 192.0.2.1:443 A cert=x\r\nobfs4 192.0.2.2:443 B\n").unwrap();
        assert_eq!(got, "obfs4 192.0.2.1:443 A cert=x\nobfs4 192.0.2.2:443 B\n");
    }

    #[test]
    fn input_over_the_limit_is_refused_whole() {
        assert!(normalize(&vec![b'a'; INPUT_LIMIT]).is_ok());
        assert!(normalize(&vec![b'a'; INPUT_LIMIT + 1]).is_err());
    }

    #[test]
    fn a_nul_is_a_rejected_control_character_not_nothing() {
        let n = normalize(b"obfs4 192.0.2.1:443 A\0B\n").unwrap();
        assert_eq!(count(&n).1, 1);
    }

    #[test]
    fn lines_are_counted_by_transport() {
        let text = "obfs4 192.0.2.1:443 AAAA cert=x iat-mode=0\nobfs4 192.0.2.2:443 BBBB cert=y iat-mode=0\n\
webtunnel [2001:db8::1]:443 CCCC url=https://example.com/x\n198.51.100.7:443 DDDD\n# a comment\n\n\
this is not a bridge\nobfs4 not-an-address\nobfs4 192.0.2.3:443 X cert=a\\\n";
        let (counts, invalid) = count(&normalize(text.as_bytes()).unwrap());
        assert_eq!(counts.get("obfs4"), Some(&2));
        assert_eq!(counts.get("webtunnel"), Some(&1));
        assert_eq!(counts.get("plain"), Some(&1));
        assert_eq!(invalid, 3, "prose, a bad address, and a trailing backslash");
    }
}
