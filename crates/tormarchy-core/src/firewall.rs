//! The kill switch: Tormarchy's own nftables table, IPv6 off while connected,
//! and the lockdown that confines browser-only browsers to loopback.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::host::{Host, DNS_PORT, TRANS_PORT};
use crate::sys;
use crate::Mode;

pub const NFT_TABLE: &str = "tormarchy";
/// Browser-only's lockdown lives in a table of its own. The main table is
/// replaced on every connect and mode change and deleted on disconnect, and none
/// of that may unlock a browser that is still running.
pub const NFT_BROWSER_TABLE: &str = "tormarchy_browser";
pub const TOR_USER: &str = "tor";
/// Tor's virtual range for .onion names resolved via AutomapHostsOnResolve.
pub const VIRT_NET: &str = "10.192.0.0/10";
/// Networks that stay reachable in lan mode. Link-local and CGNAT are included
/// because captive portals and tethered connections live there.
pub const LAN_NETS_V4: &str =
    "10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 169.254.0.0/16, 100.64.0.0/10";

/// Everything lives in our own table so the user's ufw rules are never read,
/// rewritten or flushed: a DROP in any table wins, so the kill switch holds
/// without touching theirs.
pub fn build_ruleset(mode: Mode) -> String {
    let (lan_nat, lan_filter, lan_forward) = if mode == Mode::Lan {
        (
            format!("    ip daddr {{ {LAN_NETS_V4} }} return"),
            format!("    ip daddr {{ {LAN_NETS_V4} }} accept"),
            // Routing between local machines stays possible in lan mode.
            // Forwarding anywhere else still dies: a VM reaching the internet
            // through this host is a leak in either mode.
            format!("    ip daddr {{ {LAN_NETS_V4} }} ip saddr {{ {LAN_NETS_V4} }} accept"),
        )
    } else {
        (String::new(), String::new(), String::new())
    };
    format!(
        r#"table inet {NFT_TABLE} {{
  chain nat_output {{
    type nat hook output priority dstnat; policy accept;

    # Tor's own traffic must never be redirected back into Tor.
    meta skuid "{TOR_USER}" return
    oifname "lo" return
    ip daddr 127.0.0.0/8 return

    # Onion addresses land in Tor's virtual range. This MUST precede the LAN
    # exemption: 10.192.0.0/10 sits inside 10.0.0.0/8, so a LAN return placed
    # first would send every .onion straight out of the NIC.
    ip daddr {VIRT_NET} meta l4proto tcp redirect to :{TRANS_PORT}

    # DHCP has to survive or the lease expires and the link dies with it.
    udp sport 68 udp dport 67 return

    # DNS goes to Tor's resolver BEFORE the LAN exemption. The other order is a
    # live DNS leak: the usual resolver on a home network is the router itself.
    meta l4proto {{ tcp, udp }} th dport 53 redirect to :{DNS_PORT}

{lan_nat}

    meta l4proto tcp redirect to :{TRANS_PORT}
  }}

  chain filter_output {{
    type filter hook output priority filter; policy drop;

    ct state invalid drop

    meta skuid "{TOR_USER}" accept

    # Redirected packets arrive here with a loopback destination, which is what
    # makes the default drop below safe for real traffic.
    oifname "lo" accept
    ip daddr 127.0.0.0/8 accept

    udp sport 68 udp dport 67 accept

    # Still addressed to the virtual range means NOT redirected above, since
    # that redirect is TCP-only: UDP or ICMP aimed at a resolved .onion. It dies
    # here, before the LAN exemption that would otherwise put it on the wire.
    ip daddr {VIRT_NET} drop

    # No IPv6 leaves, ever.
    meta nfproto ipv6 drop

    # DNS over TLS and over QUIC cannot be redirected into Tor's resolver, and in
    # lan mode one on the local network was accepted below. Dropped, resolvers
    # fall back to port 53, which the nat chain sends to Tor.
    meta l4proto {{ tcp, udp }} th dport 853 drop

{lan_filter}

    # Everything else: UDP, QUIC, ICMP, mDNS, NTP, and any connection opened
    # before the rules went on. This is the kill switch.
    counter drop
  }}

  chain filter_forward {{
    type filter hook forward priority filter; policy drop;

    # Traffic this machine routes for something else never passes the output
    # hook: a bridged VM, a container network, a hotspot or a tethered phone
    # would each go straight out of the NIC untouched.
{lan_forward}
    counter drop
  }}
}}
"#
    )
}

/// Replaces the table atomically: delete and re-add go in as one transaction,
/// so there is no instant where traffic is unfiltered. Checked before applied.
pub fn apply_rules(host: &dyn Host, mode: Mode) -> Result<(), String> {
    let ruleset = format!(
        "table inet {NFT_TABLE}\ndelete table inet {NFT_TABLE}\n{}",
        build_ruleset(mode)
    );
    let check = host.nft(&["-c", "-f", "-"], Some(&ruleset));
    if !check.ok() {
        return Err(first_line(&check.stderr_str(), "nft rejected the ruleset"));
    }
    let apply = host.nft(&["-f", "-"], Some(&ruleset));
    if !apply.ok() {
        return Err(first_line(
            &apply.stderr_str(),
            "nft could not apply the ruleset",
        ));
    }
    Ok(())
}

/// Idempotent: an empty table is created first, so deleting one that was not
/// there is not an error.
pub fn flush_rules(host: &dyn Host) {
    host.nft(
        &["-f", "-"],
        Some(&format!(
            "table inet {NFT_TABLE}\ndelete table inet {NFT_TABLE}\n"
        )),
    );
}

pub fn table_exists(host: &dyn Host, table: &str) -> bool {
    host.nft(&["list", "table", "inet", table], None).ok()
}

/// Deletes a table if it is there; whether it was.
pub fn delete_table(host: &dyn Host, table: &str) -> Result<bool, String> {
    if !table_exists(host, table) {
        return Ok(false);
    }
    let out = host.nft(&["delete", "table", "inet", table], None);
    if out.ok() {
        Ok(true)
    } else {
        Err(first_line(
            &out.stderr_str(),
            "nft could not delete the table",
        ))
    }
}

fn first_line(text: &str, fallback: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

/// Turns the kernel's IPv6 stack off while connected.
///
/// The ruleset already drops every IPv6 packet; this is belt and braces, since
/// with the stack down there is nothing to leak through if a rule is ever
/// mis-ordered. Written to /proc, never sysctl.d, so a reboot always restores
/// it. Every interface's own value is saved, once per connection and never
/// overwritten -- a second call would otherwise record the value this had just
/// written. Loopback is put back straight after: `all` switches it off too, and
/// that broke anything bound to ::1 for no privacy gain.
pub fn ipv6_disable(host: &dyn Host) -> Result<(), String> {
    let p = host.paths();
    let all = p.ipv6_conf.join("all/disable_ipv6");
    if fs::metadata(&all).is_err() {
        return Ok(());
    }
    sys::ensure_dir(&p.state_dir, 0o755).map_err(|e| e.to_string())?;
    if !p.ipv6_state_file.exists() {
        let mut saved = String::new();
        for (iface, value) in ipv6_values(&p.ipv6_conf) {
            saved.push_str(&format!("{iface} {value}\n"));
        }
        sys::write_atomic(&p.ipv6_state_file, saved.as_bytes(), 0o644)
            .map_err(|e| e.to_string())?;
    }
    let mut ok = fs::write(&all, "1").is_ok();
    ok &= fs::write(p.ipv6_conf.join("default/disable_ipv6"), "1").is_ok();
    let _ = fs::write(p.ipv6_conf.join("lo/disable_ipv6"), "0");
    if ok {
        Ok(())
    } else {
        Err("could not write the IPv6 switches".into())
    }
}

fn ipv6_values(conf: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Ok(dir) = fs::read_dir(conf) {
        for entry in dir.flatten() {
            let iface = entry.file_name().to_string_lossy().into_owned();
            if let Ok(v) = fs::read_to_string(entry.path().join("disable_ipv6")) {
                let v = v.trim();
                if v == "0" || v == "1" {
                    out.insert(iface, v.to_string());
                }
            }
        }
    }
    out
}

/// Puts IPv6 back exactly as it was, interface by interface. No state file
/// means this never switched IPv6 off, so there is nothing of ours to undo --
/// writing zeros regardless turned IPv6 back on for people who keep it off.
pub fn ipv6_restore(host: &dyn Host) {
    let p = host.paths();
    let Some(saved_text) = sys::read_bounded_string(&p.ipv6_state_file, 64 * 1024) else {
        return;
    };
    let mut saved: BTreeMap<String, String> = BTreeMap::new();
    for line in saved_text.lines() {
        let mut f = line.split_whitespace();
        match (f.next(), f.next()) {
            // An older state file held one bare value for all and default.
            (Some(v), None) if v == "0" || v == "1" => {
                saved.insert("all".into(), v.into());
                saved.insert("default".into(), v.into());
            }
            (Some(iface), Some(v))
                if (v == "0" || v == "1")
                    && !iface.is_empty()
                    && iface
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "_.:@-".contains(c)) =>
            {
                saved.insert(iface.into(), v.into());
            }
            _ => {}
        }
    }
    let get = |k: &str| saved.get(k).cloned();
    let default = get("default").unwrap_or_else(|| "0".into());
    // all first, because writing it resets every interface.
    let _ = fs::write(
        p.ipv6_conf.join("all/disable_ipv6"),
        get("all").unwrap_or_else(|| "0".into()),
    );
    let _ = fs::write(p.ipv6_conf.join("default/disable_ipv6"), &default);
    for iface in ipv6_values(&p.ipv6_conf).keys() {
        if iface == "all" || iface == "default" {
            continue;
        }
        let value = get(iface).unwrap_or_else(|| default.clone());
        let _ = fs::write(p.ipv6_conf.join(iface).join("disable_ipv6"), value);
    }
    sys::remove_file_quiet(&p.ipv6_state_file);
}

/// Whether a cgroup path is a browser scope inside its own user's manager.
pub fn is_browser_scope(path: &str, uid: Option<u32>) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    let (user_slice, manager, rest) = match parts.as_slice() {
        ["user.slice", us, mgr, rest @ ..] => (*us, *mgr, rest),
        _ => return false,
    };
    let Some(a) = user_slice
        .strip_prefix("user-")
        .and_then(|s| s.strip_suffix(".slice"))
    else {
        return false;
    };
    let Some(b) = manager
        .strip_prefix("user@")
        .and_then(|s| s.strip_suffix(".service"))
    else {
        return false;
    };
    if a != b || a.is_empty() || !a.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    if let Some(uid) = uid {
        if a != uid.to_string() {
            return false;
        }
    }
    let scope = match rest {
        [scope] => *scope,
        [slice, scope] => {
            let ok = slice
                .strip_suffix(".slice")
                .map(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c == '-'))
                .unwrap_or(false);
            if !ok {
                return false;
            }
            *scope
        }
        _ => return false,
    };
    scope
        .strip_prefix("tormarchy-browser-")
        .and_then(|s| s.strip_suffix(".scope"))
        .map(|s| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
        })
        .unwrap_or(false)
}

/// Every browser scope that exists now, the caller's first.
pub fn browser_scopes(cgroup_root: &Path, caller: &str) -> Vec<String> {
    let mut out = vec![caller.to_string()];
    let Ok(users) = fs::read_dir(cgroup_root.join("user.slice")) else {
        return out;
    };
    for user in users.flatten() {
        let Ok(managers) = fs::read_dir(user.path()) else {
            continue;
        };
        for manager in managers.flatten() {
            let mut dirs = vec![manager.path()];
            if let Ok(inner) = fs::read_dir(manager.path()) {
                dirs.extend(
                    inner
                        .flatten()
                        .map(|e| e.path())
                        .filter(|p| p.extension().map(|e| e == "slice").unwrap_or(false)),
                );
            }
            for dir in dirs {
                let Ok(entries) = fs::read_dir(&dir) else {
                    continue;
                };
                for entry in entries.flatten() {
                    if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        continue;
                    }
                    let Ok(rel) = entry
                        .path()
                        .strip_prefix(cgroup_root)
                        .map(|r| r.to_string_lossy().into_owned())
                    else {
                        continue;
                    };
                    if rel != caller && is_browser_scope(&rel, None) && !out.contains(&rel) {
                        out.push(rel);
                    }
                }
            }
        }
    }
    out
}

/// The browser table with one jump to lockdown per scope given, replacing what
/// was there in the same transaction.
///
/// Loopback is all a locked browser may reach, and port 53 not even there: the
/// system's stub resolver lives on loopback and forwards whatever it is asked
/// to the real DNS servers.
pub fn browser_ruleset(scopes: &[String]) -> String {
    let t = NFT_BROWSER_TABLE;
    let mut s = format!(
        "table inet {t} {{\n  chain output {{\n    type filter hook output priority filter; policy accept;\n  }}\n  chain lockdown {{\n  }}\n}}\n\
flush chain inet {t} lockdown\n\
add rule inet {t} lockdown meta l4proto {{ tcp, udp }} th dport 53 drop\n\
add rule inet {t} lockdown oifname \"lo\" accept\n\
add rule inet {t} lockdown counter drop\n\
flush chain inet {t} output\n"
    );
    for scope in scopes {
        let level = scope.split('/').count();
        s.push_str(&format!(
            "add rule inet {t} output socket cgroupv2 level {level} \"{scope}\" jump lockdown\n"
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dns_and_onions_precede_the_lan_exemption() {
        let r = build_ruleset(Mode::Lan);
        let at = |needle: &str| r.find(needle).unwrap_or_else(|| panic!("missing {needle}"));
        let lan_return = at(&format!("ip daddr {{ {LAN_NETS_V4} }} return"));
        assert!(
            at("th dport 53 redirect") < lan_return,
            "DNS must be redirected before the LAN return"
        );
        assert!(at(&format!("ip daddr {VIRT_NET} meta l4proto tcp redirect")) < lan_return);
        let lan_accept = at(&format!("ip daddr {{ {LAN_NETS_V4} }} accept"));
        assert!(at(&format!("ip daddr {VIRT_NET} drop")) < lan_accept);
        assert!(
            at("th dport 853 drop") < lan_accept,
            "DNS over TLS dies before the LAN accept"
        );
        assert!(at("meta nfproto ipv6 drop") < lan_accept);
    }

    #[test]
    fn strict_has_no_lan_exemption() {
        let r = build_ruleset(Mode::Strict);
        assert!(!r.contains("192.168.0.0/16"));
        assert!(r.contains("policy drop"));
        assert!(r.contains("chain filter_forward"));
    }

    #[test]
    fn tor_itself_is_exempt_first() {
        let r = build_ruleset(Mode::Strict);
        assert!(
            r.find("meta skuid \"tor\" return").unwrap() < r.find("redirect to :9040").unwrap()
        );
    }

    #[test]
    fn browser_scopes_are_recognised_strictly() {
        let ok = "user.slice/user-1000.slice/user@1000.service/app.slice/tormarchy-browser-firefox-41.scope";
        assert!(is_browser_scope(ok, Some(1000)));
        assert!(!is_browser_scope(ok, Some(1001)), "another user's scope");
        assert!(is_browser_scope(
            "user.slice/user-1000.slice/user@1000.service/tormarchy-browser-x-1.scope",
            None
        ));
        assert!(!is_browser_scope(
            "user.slice/user-1000.slice/user@1001.service/app.slice/tormarchy-browser-x-1.scope",
            None
        ));
        assert!(!is_browser_scope(
            "user.slice/user-1000.slice/user@1000.service/app.slice/firefox.scope",
            None
        ));
        assert!(!is_browser_scope(
            "system.slice/tormarchy-browser-x.scope",
            None
        ));
        assert!(!is_browser_scope(
            "user.slice/user-1000.slice/user@1000.service/app.slice/tormarchy-browser-a\"b.scope",
            None
        ));
    }

    #[test]
    fn the_browser_set_is_replaced_with_one_rule_per_live_scope() {
        let root = sys::create_temp_dir("tm-cg-").unwrap();
        let mine = "user.slice/user-1000.slice/user@1000.service/app.slice/tormarchy-browser-firefox-41.scope";
        let other =
            "user.slice/user-1000.slice/user@1000.service/tormarchy-browser-chromium-7.scope";
        let foreign =
            "user.slice/user-1000.slice/user@1001.service/app.slice/tormarchy-browser-x-1.scope";
        let unrelated = "user.slice/user-1000.slice/user@1000.service/app.slice/firefox.scope";
        for d in [mine, other, foreign, unrelated] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        let scopes = browser_scopes(&root, mine);
        assert_eq!(scopes, vec![mine.to_string(), other.to_string()]);
        let rules = browser_ruleset(&scopes);
        assert_eq!(rules.matches("jump lockdown").count(), 2);
        assert!(rules.contains(&format!("flush chain inet {NFT_BROWSER_TABLE} output")));
        assert!(rules.contains(&format!("level 5 \"{mine}\"")));
        fs::remove_dir_all(root).unwrap();
    }
}
