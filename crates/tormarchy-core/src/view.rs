//! What the panel shows, in words: every label, verdict and tooltip, and which
//! changes deserve a notification. Pure functions, so every one is tested
//! without a shell. (This was Model.js.)

use serde::Serialize;

use crate::doctor::{Check, Status};
use crate::tor::Hop;
use crate::Mode;

/// Exit-country names. Tor reports two-letter codes; a panel showing "de"
/// where it could show "Germany" is leaking implementation detail.
const COUNTRY_NAMES: &[(&str, &str)] = &[
    ("ad", "Andorra"),
    ("ae", "United Arab Emirates"),
    ("al", "Albania"),
    ("am", "Armenia"),
    ("ar", "Argentina"),
    ("at", "Austria"),
    ("au", "Australia"),
    ("az", "Azerbaijan"),
    ("ba", "Bosnia and Herzegovina"),
    ("bd", "Bangladesh"),
    ("be", "Belgium"),
    ("bg", "Bulgaria"),
    ("bh", "Bahrain"),
    ("bo", "Bolivia"),
    ("br", "Brazil"),
    ("by", "Belarus"),
    ("ca", "Canada"),
    ("ch", "Switzerland"),
    ("cl", "Chile"),
    ("cn", "China"),
    ("co", "Colombia"),
    ("cr", "Costa Rica"),
    ("cy", "Cyprus"),
    ("cz", "Czechia"),
    ("de", "Germany"),
    ("dk", "Denmark"),
    ("do", "Dominican Republic"),
    ("dz", "Algeria"),
    ("ec", "Ecuador"),
    ("ee", "Estonia"),
    ("eg", "Egypt"),
    ("es", "Spain"),
    ("et", "Ethiopia"),
    ("fi", "Finland"),
    ("fr", "France"),
    ("gb", "United Kingdom"),
    ("ge", "Georgia"),
    ("gh", "Ghana"),
    ("gr", "Greece"),
    ("gt", "Guatemala"),
    ("hk", "Hong Kong"),
    ("hn", "Honduras"),
    ("hr", "Croatia"),
    ("hu", "Hungary"),
    ("id", "Indonesia"),
    ("ie", "Ireland"),
    ("il", "Israel"),
    ("in", "India"),
    ("iq", "Iraq"),
    ("ir", "Iran"),
    ("is", "Iceland"),
    ("it", "Italy"),
    ("jm", "Jamaica"),
    ("jo", "Jordan"),
    ("jp", "Japan"),
    ("ke", "Kenya"),
    ("kg", "Kyrgyzstan"),
    ("kh", "Cambodia"),
    ("kr", "South Korea"),
    ("kw", "Kuwait"),
    ("kz", "Kazakhstan"),
    ("lb", "Lebanon"),
    ("li", "Liechtenstein"),
    ("lk", "Sri Lanka"),
    ("lt", "Lithuania"),
    ("lu", "Luxembourg"),
    ("lv", "Latvia"),
    ("ma", "Morocco"),
    ("md", "Moldova"),
    ("me", "Montenegro"),
    ("mk", "North Macedonia"),
    ("mn", "Mongolia"),
    ("mt", "Malta"),
    ("mu", "Mauritius"),
    ("mx", "Mexico"),
    ("my", "Malaysia"),
    ("mz", "Mozambique"),
    ("ng", "Nigeria"),
    ("nl", "Netherlands"),
    ("no", "Norway"),
    ("np", "Nepal"),
    ("nz", "New Zealand"),
    ("om", "Oman"),
    ("pa", "Panama"),
    ("pe", "Peru"),
    ("ph", "Philippines"),
    ("pk", "Pakistan"),
    ("pl", "Poland"),
    ("pr", "Puerto Rico"),
    ("pt", "Portugal"),
    ("py", "Paraguay"),
    ("qa", "Qatar"),
    ("ro", "Romania"),
    ("rs", "Serbia"),
    ("ru", "Russia"),
    ("sa", "Saudi Arabia"),
    ("se", "Sweden"),
    ("sg", "Singapore"),
    ("si", "Slovenia"),
    ("sk", "Slovakia"),
    ("sn", "Senegal"),
    ("sv", "El Salvador"),
    ("th", "Thailand"),
    ("tn", "Tunisia"),
    ("tr", "Türkiye"),
    ("tw", "Taiwan"),
    ("tz", "Tanzania"),
    ("ua", "Ukraine"),
    ("ug", "Uganda"),
    ("us", "United States"),
    ("uy", "Uruguay"),
    ("uz", "Uzbekistan"),
    ("ve", "Venezuela"),
    ("vn", "Vietnam"),
    ("za", "South Africa"),
    ("zw", "Zimbabwe"),
];

pub fn country_name(code: &str) -> String {
    let cc = code.to_ascii_lowercase();
    if cc.is_empty() {
        return String::new();
    }
    COUNTRY_NAMES
        .iter()
        .find(|(c, _)| *c == cc)
        .map(|(_, n)| n.to_string())
        .unwrap_or_else(|| cc.to_uppercase())
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn elide(text: &str, limit: usize) -> String {
    let v = collapse(text);
    if v.chars().count() > limit {
        let head: String = v.chars().take(limit.saturating_sub(3)).collect();
        format!("{head}…")
    } else {
        v
    }
}

pub const BRIEF_LIMIT: usize = 32;

/// The first clause of a message, capped: "tor stalled at 40% bootstrap. No
/// rules applied, ..." reads "Tor stalled at 40% bootstrap".
pub fn brief(text: &str, limit: usize) -> String {
    let mut s = collapse(text);
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("tormarchy:") {
        s = s["tormarchy:".len()..].trim_start().to_string();
    }
    let cut = [". ", " -- ", "; ", ": "]
        .iter()
        .filter_map(|p| s.find(p))
        .filter(|i| *i > 0)
        .min();
    if let Some(i) = cut {
        s.truncate(i);
    }
    while s.ends_with(['.', ':', ';']) {
        s.pop();
    }
    if s.chars().count() > limit {
        let head: String = s.chars().take(limit - 1).collect();
        let space = head.rfind(' ');
        s = match space {
            Some(sp) if sp > limit / 2 => head[..sp].to_string(),
            _ => head,
        };
        s.push('…');
    }
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Word-wrapped to lines a bubble can hold: the shell's tooltip draws a single
/// line however long it is.
pub fn wrap_tip(text: &str, width: usize) -> String {
    let mut out: Vec<String> = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split_whitespace() {
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
                out.push(std::mem::take(&mut line));
                line.push_str(word);
            } else {
                if !line.is_empty() {
                    line.push(' ');
                }
                line.push_str(word);
            }
        }
        if !line.is_empty() {
            out.push(line);
        }
    }
    out.join("\n")
}

pub fn tip(text: &str) -> String {
    wrap_tip(text, 46)
}

/// A status warning as a label. The two the helper sends get their own words.
pub fn brief_warning(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    if lower.contains("kill switch") {
        return "Kill switch holding".into();
    }
    if let Some(i) = lower.find("bootstrapping at ") {
        let digits: String = lower[i + 17..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if !digits.is_empty() && lower[i + 17 + digits.len()..].starts_with('%') {
            return format!("Reconnecting \u{b7} {digits}%");
        }
    }
    brief(text, BRIEF_LIMIT)
}

pub fn format_age(seconds: u64) -> String {
    match seconds {
        n if n < 60 => format!("{n}s"),
        n if n < 3600 => format!("{}m {}s", n / 60, n % 60),
        n => format!("{}h {}m", n / 3600, (n % 3600) / 60),
    }
}

pub fn format_ago(seconds: u64) -> String {
    match seconds {
        n if n < 60 => "just now".into(),
        n if n < 3600 => format!("{} min ago", n / 60),
        n => format!("{} h ago", n / 3600),
    }
}

/// Raw byte totals: whole bytes, one decimal above that.
pub fn format_bytes(bytes: u64) -> String {
    if bytes == 0 {
        return "0 B".into();
    }
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut n = bytes as f64;
    let mut i = 0;
    while n >= 1024.0 && i < units.len() - 1 {
        n /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{} {}", n.round(), units[i])
    } else if n < 10.0 {
        format!("{:.1} {}", n, units[i])
    } else {
        format!("{:.0} {}", n, units[i])
    }
}

/// A dash, never "0 ms": zero is a measurement, and claiming one while nothing
/// has been measured states something false.
pub fn format_latency(ms: u64) -> String {
    match ms {
        0 => "— ms".into(),
        n if n < 1000 => format!("{n} ms"),
        n => format!("{:.1} s", n as f64 / 1000.0),
    }
}

/// Where a latency sits on the gauge, anchored at 3 s: slower than that is
/// effectively unusable.
pub fn latency_fraction(ms: u64) -> f64 {
    if ms == 0 {
        0.0
    } else {
        (ms as f64 / 3000.0).min(1.0)
    }
}

/// A number alone does not say whether 900 ms is normal for Tor (it is).
pub fn latency_verdict(ms: u64) -> &'static str {
    match ms {
        0 => "",
        n if n < 400 => "Fast for Tor",
        n if n < 900 => "Normal for Tor",
        n if n < 2000 => "Slow — try a new circuit",
        _ => "Very slow — try a new circuit",
    }
}

pub fn format_relay_bandwidth(kbps: u64) -> String {
    match kbps {
        0 => String::new(),
        n if n < 1000 => format!("{n} KB/s"),
        n => {
            let mb = n as f64 / 1000.0;
            if mb < 10.0 {
                format!("{mb:.1} MB/s")
            } else {
                format!("{:.0} MB/s", mb)
            }
        }
    }
}

pub fn hop_role(index: usize, total: usize) -> &'static str {
    if total == 0 {
        ""
    } else if index == 0 {
        "Guard"
    } else if index + 1 == total {
        "Exit"
    } else {
        "Middle"
    }
}

/// "Guard · Sweden · relayname · 12 MB/s · Fast, Stable". A flag repeating the
/// role is left out.
pub fn hop_tooltip(hop: &Hop, index: usize, total: usize) -> String {
    let role = hop_role(index, total);
    let mut parts = vec![role.to_string()];
    if !hop.cc.is_empty() && hop.cc != "xx" {
        parts.push(country_name(&hop.cc));
    }
    if !hop.nickname.is_empty() {
        parts.push(hop.nickname.clone());
    }
    let bw = format_relay_bandwidth(hop.bandwidth_kbps);
    if !bw.is_empty() {
        parts.push(bw);
    }
    let flags: Vec<&str> = hop
        .flags
        .iter()
        .map(String::as_str)
        .filter(|f| *f != role)
        .collect();
    if !flags.is_empty() {
        parts.push(flags.join(", "));
    }
    parts.join(" · ")
}

pub fn hop_label(cc: &str) -> String {
    if cc.is_empty() || cc == "xx" {
        "??".into()
    } else {
        cc.to_uppercase()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Choice {
    pub value: String,
    pub label: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// What the exit picker's trigger shows: the pinned country, or on Automatic
/// with countries excluded, "Not US, RU".
pub fn exit_label(requested: Option<&str>, excluded: &[String]) -> String {
    match requested {
        Some(cc) if !cc.is_empty() => country_name(cc),
        _ if !excluded.is_empty() => format!("Not {}", excluded.join(", ").to_uppercase()),
        _ => "Automatic".into(),
    }
}

/// Automatic, then countries picked recently, then everything else
/// alphabetically. Excluded countries and recent ones with no exits right now
/// are left out; the pinned one is always there, so the trigger has a label.
pub fn exit_options(
    codes: &[String],
    recent: &[String],
    excluded: &[String],
    requested: Option<&str>,
) -> Vec<Choice> {
    let mut out = vec![Choice {
        value: "auto".into(),
        label: exit_label(None, excluded),
        description: if excluded.is_empty() {
            "Let Tor choose the exit".into()
        } else {
            "Tor chooses from everywhere else".into()
        },
    }];
    let mut available: Vec<String> = Vec::new();
    for c in codes {
        let cc = c.to_ascii_lowercase();
        if cc.len() == 2 && !available.contains(&cc) && !excluded.contains(&cc) {
            available.push(cc);
        }
    }
    let mut first: Vec<Choice> = Vec::new();
    for r in recent {
        let r = r.to_ascii_lowercase();
        if available.contains(&r) && !first.iter().any(|c| c.value == r) {
            first.push(Choice {
                value: r.clone(),
                label: country_name(&r),
                description: "Recent".into(),
            });
        }
    }
    let mut rest: Vec<Choice> = available
        .iter()
        .filter(|cc| !first.iter().any(|c| &c.value == *cc))
        .map(|cc| Choice {
            value: cc.clone(),
            label: country_name(cc),
            description: String::new(),
        })
        .collect();
    rest.sort_by(|a, b| a.label.cmp(&b.label));
    if let Some(cur) = requested.map(str::to_ascii_lowercase) {
        if cur.len() == 2
            && cur.chars().all(|c| c.is_ascii_lowercase())
            && !available.contains(&cur)
        {
            first.insert(
                0,
                Choice {
                    value: cur.clone(),
                    label: country_name(&cur),
                    description: String::new(),
                },
            );
        }
    }
    out.extend(first);
    out.extend(rest);
    out
}

pub fn doctor_glyph(status: Option<Status>) -> &'static str {
    match status {
        Some(Status::Pass) => "󰄬",
        Some(Status::Warn) => "󰀦",
        Some(Status::Fail) => "󰅙",
        _ => "–",
    }
}

/// The worst result, for the tile's one glyph.
pub fn doctor_overall(checks: &[Check]) -> Status {
    if checks.iter().any(|c| c.status == Status::Fail) {
        Status::Fail
    } else if checks.iter().any(|c| c.status == Status::Warn) {
        Status::Warn
    } else {
        Status::Pass
    }
}

/// "5 passed · 1 warning". Skips are what the mode does not need checking.
pub fn doctor_summary(checks: &[Check]) -> String {
    let n = |s: Status| checks.iter().filter(|c| c.status == s).count();
    let (passed, warned, failed) = (n(Status::Pass), n(Status::Warn), n(Status::Fail));
    let mut parts = Vec::new();
    if failed > 0 {
        parts.push(if failed == 1 {
            "1 problem".to_string()
        } else {
            format!("{failed} problems")
        });
    }
    if warned > 0 {
        parts.push(if warned == 1 {
            "1 warning".to_string()
        } else {
            format!("{warned} warnings")
        });
    }
    if passed > 0 {
        parts.push(format!("{passed} passed"));
    }
    if parts.is_empty() {
        "Nothing to check".into()
    } else {
        parts.join(" · ")
    }
}

pub fn check_label(c: &Check) -> String {
    match c.id.as_str() {
        "tor" => "Tor".into(),
        "bootstrap" => "Bootstrap".into(),
        "control" => "Control port".into(),
        "proxy" => "Proxy".into(),
        "routing" => "Routing".into(),
        "exit" => "Exit".into(),
        "ipv6" => "IPv6".into(),
        "ipv6-stack" => "IPv6 stack".into(),
        "dns" => "DNS".into(),
        "forwarding" => "Forwarding".into(),
        "clock" => "Clock".into(),
        _ => brief(&c.text, 16),
    }
}

pub fn check_tip(c: &Check) -> String {
    if c.detail.is_empty() {
        tip(&c.text)
    } else {
        tip(&format!("{}\n{}", c.text, c.detail))
    }
}

/// Bridges, as the tile states them. The transport only when there is just
/// one; the full breakdown is in the expanded section.
pub fn bridge_state_label(state: &str, count: usize, transports: &[String]) -> String {
    let what = if count == 0 {
        String::new()
    } else if transports.len() == 1 {
        format!("{count} {}", transports[0])
    } else if count == 1 {
        "1 bridge".into()
    } else {
        format!("{count} bridges")
    };
    match state {
        "active" if what.is_empty() => "On".into(),
        "active" => format!("On · {what}"),
        "stashed" if count > 0 => format!("Off · {count} saved"),
        "stashed" => "Off · saved".into(),
        _ => "Off".into(),
    }
}

pub fn bridge_count_text(count: usize) -> String {
    match count {
        0 => "No bridge lines".into(),
        1 => "1 bridge".into(),
        n => format!("{n} bridges"),
    }
}

// ---- notifications --------------------------------------------------------
//
// The states are the bar icon's: off, connecting, routed, browser, blocked.
// "problems" (the default) speaks only when something has gone wrong or come
// back; "all" also confirms the slow successes; "off" never speaks.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    Problems,
    All,
    Off,
}

impl Policy {
    pub fn parse(s: &str) -> Policy {
        match s {
            "all" => Policy::All,
            "off" => Policy::Off,
            _ => Policy::Problems,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Conn {
    Off,
    Connecting,
    Routed,
    Browser,
    Blocked,
}

pub fn connection_state(
    installed: bool,
    connected: bool,
    tor_running: bool,
    mode: Option<Mode>,
    bootstrapping: bool,
) -> Conn {
    if !installed {
        Conn::Off
    } else if connected && !tor_running {
        Conn::Blocked
    } else if connected {
        Conn::Routed
    } else if mode == Some(Mode::Socks) && tor_running && !bootstrapping {
        Conn::Browser
    } else if tor_running {
        Conn::Connecting
    } else {
        Conn::Off
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub headline: String,
    pub body: String,
    pub urgency: &'static str,
    pub glyph: &'static str,
}

fn note(headline: &str, body: &str, urgency: &'static str, glyph: &'static str) -> Note {
    Note {
        headline: headline.into(),
        body: body.into(),
        urgency,
        glyph,
    }
}

pub fn notification_for(
    prev: Conn,
    next: Conn,
    policy: Policy,
    exit_country_name: &str,
) -> Option<Note> {
    if policy == Policy::Off || prev == next {
        return None;
    }
    // The one that matters most: critical, so it stays until dealt with.
    if next == Conn::Blocked {
        return Some(note(
            "Kill switch engaged",
            "Network blocked until Tor is back",
            "critical",
            "󰻌",
        ));
    }
    // Leaving it replaces that toast, so a critical one never outlives the
    // problem it describes.
    if prev == Conn::Blocked && next == Conn::Routed {
        return Some(note(
            "Tor is back",
            "Traffic is routed again",
            "normal",
            "󰕥",
        ));
    }
    if prev == Conn::Blocked {
        return Some(note("Network restored", "Tormarchy is off", "low", "󰦞"));
    }
    if policy != Policy::All {
        return None;
    }
    match next {
        Conn::Routed => Some(note(
            "Connected through Tor",
            &if exit_country_name.is_empty() {
                "All traffic via Tor".to_string()
            } else {
                format!("Leaving from {exit_country_name}")
            },
            "low",
            "󰕥",
        )),
        Conn::Browser => Some(note("Tor is ready", "Browser-only proxy is up", "low", "󰇧")),
        _ => None,
    }
}

/// A connect that failed on its own. Not one the user cancelled: they know.
pub fn connect_failed(error: &str, policy: Policy) -> Option<Note> {
    if policy == Policy::Off {
        return None;
    }
    let b = brief(error, 40);
    Some(note(
        "Couldn't connect to Tor",
        if b.is_empty() {
            "Network unchanged"
        } else {
            &b
        },
        "normal",
        "󰀨",
    ))
}

/// Bridges switched on by connect's own fallback, rather than by the user.
pub fn bridges_switched_on(
    prev: &str,
    next: &str,
    policy: Policy,
    user_action: bool,
) -> Option<Note> {
    if policy != Policy::All || user_action || next != "active" || prev == "active" {
        return None;
    }
    Some(note("Using bridges", "This network blocks Tor", "low", "󰘘"))
}

/// Discrete arguments through Omarchy's own sender, never a shell string and
/// never notify-send. The headline and body go in as positionals.
pub fn notification_argv(n: &Note, replace_id: Option<&str>) -> Vec<String> {
    let mut argv: Vec<String> = [
        "omarchy-notification-send",
        "--app-name",
        "Tormarchy",
        "-g",
        n.glyph,
        "-u",
        n.urgency,
        "-p",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if let Some(id) = replace_id.filter(|i| !i.is_empty() && i.chars().all(|c| c.is_ascii_digit()))
    {
        argv.push("-r".into());
        argv.push(id.into());
    }
    argv.extend([
        n.headline.clone(),
        n.body.clone(),
        "--exec".into(),
        "omarchy-shell".into(),
        "tripleu.tor".into(),
        "open".into(),
    ]);
    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(id: &str, status: Status) -> Check {
        Check {
            id: id.into(),
            group: String::new(),
            status,
            text: format!("{id} text"),
            detail: String::new(),
        }
    }

    #[test]
    fn briefs_cut_to_the_first_clause() {
        assert_eq!(brief("tormarchy: tor stalled at 40% bootstrap. No rules applied, networking is unchanged.", 32), "Tor stalled at 40% bootstrap");
        let long = brief(
            "could not reach tor's control port on 127.0.0.1:9051: Permission denied",
            32,
        );
        assert!(long.chars().count() <= 32 && long.ends_with('…'), "{long}");
        assert_eq!(brief("", 32), "");
    }

    #[test]
    fn warnings_become_labels() {
        assert_eq!(
            brief_warning("Tor has stopped, so the kill switch is holding the network shut."),
            "Kill switch holding"
        );
        assert_eq!(
            brief_warning("Tor is still bootstrapping at 45%."),
            "Reconnecting · 45%"
        );
    }

    #[test]
    fn tips_wrap_without_losing_a_word() {
        let t = "Asks check.torproject.org whether you are coming from Tor, tries an IPv6 host directly, and checks DNS.";
        let w = wrap_tip(t, 46);
        assert!(w.lines().all(|l| l.chars().count() <= 46));
        assert_eq!(
            w.split_whitespace().collect::<Vec<_>>(),
            t.split_whitespace().collect::<Vec<_>>()
        );
    }

    #[test]
    fn numbers() {
        assert_eq!(format_latency(0), "— ms");
        assert_eq!(format_latency(612), "612 ms");
        assert_eq!(format_latency(2300), "2.3 s");
        assert_eq!(latency_fraction(6000), 1.0);
        assert_eq!(latency_verdict(850), "Normal for Tor");
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(4_400_000), "4.2 MB");
        assert_eq!(format_bytes(63_488), "62 KB");
        assert_eq!(format_age(15), "15s");
        assert_eq!(format_age(125), "2m 5s");
        assert_eq!(format_age(7300), "2h 1m");
        assert_eq!(format_ago(130), "2 min ago");
    }

    #[test]
    fn hop_tooltips_skip_a_flag_that_repeats_the_role() {
        let hop = Hop {
            cc: "se".into(),
            nickname: "relay".into(),
            flags: vec!["Guard".into(), "Fast".into()],
            bandwidth_kbps: 12000,
            ip: String::new(),
        };
        assert_eq!(
            hop_tooltip(&hop, 0, 3),
            "Guard · Sweden · relay · 12 MB/s · Fast"
        );
        assert_eq!(hop_label("xx"), "??");
    }

    #[test]
    fn exit_options_order() {
        let codes: Vec<String> = ["de", "nl", "us", "se", "ch"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let o = exit_options(
            &codes,
            &["se".into(), "zz".into()],
            &["us".into()],
            Some("fr"),
        );
        let values: Vec<&str> = o.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(values, ["auto", "fr", "se", "de", "nl", "ch"]);
        assert_eq!(o[0].label, "Not US");
        assert_eq!(o[2].description, "Recent");
    }

    #[test]
    fn leak_summaries() {
        let checks = vec![
            check("tor", Status::Pass),
            check("dns", Status::Warn),
            check("x", Status::Skip),
        ];
        assert_eq!(doctor_summary(&checks), "1 warning · 1 passed");
        assert_eq!(doctor_overall(&checks), Status::Warn);
        assert_eq!(
            check_label(&check("ipv6-stack", Status::Pass)),
            "IPv6 stack"
        );
        assert_eq!(
            check_label(&check("brand-new", Status::Pass)),
            "Brand-new text"
        );
    }

    #[test]
    fn bridge_labels() {
        assert_eq!(
            bridge_state_label("active", 2, &["obfs4".into()]),
            "On · 2 obfs4"
        );
        assert_eq!(
            bridge_state_label("active", 3, &["obfs4".into(), "plain".into()]),
            "On · 3 bridges"
        );
        assert_eq!(bridge_state_label("stashed", 2, &[]), "Off · 2 saved");
        assert_eq!(bridge_state_label("off", 0, &[]), "Off");
    }

    #[test]
    fn notifications() {
        use Conn::*;
        let p = Policy::Problems;
        assert_eq!(
            notification_for(Routed, Blocked, p, "").unwrap().urgency,
            "critical"
        );
        assert_eq!(
            notification_for(Blocked, Routed, p, "").unwrap().headline,
            "Tor is back"
        );
        assert_eq!(
            notification_for(Blocked, Off, p, "").unwrap().headline,
            "Network restored"
        );
        assert!(
            notification_for(Off, Routed, p, "").is_none(),
            "a connect is not news by default"
        );
        assert_eq!(
            notification_for(Off, Routed, Policy::All, "Germany")
                .unwrap()
                .body,
            "Leaving from Germany"
        );
        assert!(notification_for(Routed, Blocked, Policy::Off, "").is_none());
        assert!(bridges_switched_on("off", "active", Policy::All, true).is_none());
        assert!(bridges_switched_on("off", "active", Policy::All, false).is_some());
        assert_eq!(
            connect_failed("tormarchy: tor stalled at 40% bootstrap. x", p)
                .unwrap()
                .body,
            "Tor stalled at 40% bootstrap"
        );
    }

    #[test]
    fn notification_argv_is_discrete_and_ends_in_the_open_action() {
        let n = notification_for(Conn::Routed, Conn::Blocked, Policy::Problems, "").unwrap();
        let a = notification_argv(&n, Some("7"));
        assert_eq!(a[0], "omarchy-notification-send");
        assert!(a.windows(2).any(|w| w == ["-r", "7"]));
        assert_eq!(
            &a[a.len() - 4..],
            ["--exec", "omarchy-shell", "tripleu.tor", "open"]
        );
        assert!(!notification_argv(&n, Some("7; rm -rf /")).contains(&"-r".to_string()));
    }

    #[test]
    fn states() {
        assert_eq!(
            connection_state(true, true, false, Some(Mode::Lan), false),
            Conn::Blocked
        );
        assert_eq!(
            connection_state(true, false, true, Some(Mode::Socks), false),
            Conn::Browser
        );
        assert_eq!(
            connection_state(true, false, true, Some(Mode::Socks), true),
            Conn::Connecting
        );
        assert_eq!(connection_state(false, true, true, None, false), Conn::Off);
    }
}
