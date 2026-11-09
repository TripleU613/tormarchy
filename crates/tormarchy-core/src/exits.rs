//! Which exits tor may use: pinned to one country, or anywhere but some.

use crate::host::Host;
use crate::paths::Paths;
use crate::sys;
use crate::{Fail, Res};

pub fn valid_code(cc: &str) -> Option<String> {
    let cc = cc.trim();
    (cc.len() == 2 && cc.chars().all(|c| c.is_ascii_alphabetic())).then(|| cc.to_ascii_lowercase())
}

/// The country exits are pinned to.
pub fn requested(paths: &Paths) -> Option<String> {
    let text = sys::read_bounded_string(&paths.torrc_exit, 4096)?;
    text.lines().find_map(|l| {
        let rest = l.trim_start().strip_prefix("ExitNodes")?.trim_start();
        let inner = rest.strip_prefix('{')?.split('}').next()?;
        valid_code(inner)
    })
}

/// Countries exits must never be in.
pub fn excluded(paths: &Paths) -> Vec<String> {
    let Some(text) = sys::read_bounded_string(&paths.torrc_exclude, 4096) else {
        return vec![];
    };
    let Some(list) = text.lines().find_map(|l| {
        l.trim_start()
            .strip_prefix("ExcludeExitNodes")
            .map(str::trim)
    }) else {
        return vec![];
    };
    let mut out = Vec::new();
    for item in list.split(',') {
        if let Some(cc) = item
            .trim()
            .strip_prefix('{')
            .and_then(|s| s.strip_suffix('}'))
            .and_then(valid_code)
        {
            if !out.contains(&cc) {
                out.push(cc);
            }
        }
    }
    out
}

/// After a change to which exits are allowed. A reload changes which exits new
/// circuits may use, but the circuit already carrying traffic keeps its old exit
/// until it ages out, so NEWNYM moves new streams onto one built under the rule.
fn reload(host: &dyn Host) -> Res {
    if !host.tor_running() {
        return Ok(());
    }
    if !host.systemctl(&["reload", "tor.service"]) && !host.systemctl(&["restart", "tor.service"]) {
        return Err(Fail("wrote the config but could not reload tor".into()));
    }
    host.publish_cookie();
    let _ = host.newnym();
    Ok(())
}

/// `exit <cc>` or `exit auto`; the message to print.
pub fn pin(host: &dyn Host, target: &str) -> Res<String> {
    let paths = host.paths();
    let target = if target == "auto" {
        None
    } else {
        Some(
            valid_code(target)
                .ok_or("expected a two-letter country code, 'auto', --list, or --exclude")?,
        )
    };
    if !paths.torrc_dir.is_dir() {
        return Err(Fail(format!(
            "{} is missing -- run setup first",
            paths.torrc_dir.display()
        )));
    }
    match &target {
        Some(cc) => {
            if excluded(paths).contains(cc) {
                return Err(Fail(format!(
                    "{} is excluded, so exits cannot be pinned there. Run 'tormarchy exit --exclude none' first.",
                    cc.to_uppercase()
                )));
            }
            let conf = format!("# Managed by tormarchy. Remove this file or run 'tormarchy exit auto'.\nExitNodes {{{cc}}}\nStrictNodes 1\n");
            sys::write_atomic(&paths.torrc_exit, conf.as_bytes(), 0o644)
                .map_err(|e| Fail(e.to_string()))?;
        }
        None => sys::remove_file_quiet(&paths.torrc_exit),
    }
    reload(host)?;
    Ok(match target {
        Some(cc) => format!(
            "Exits pinned to {}. New circuits will use it.",
            cc.to_uppercase()
        ),
        None => "Exit country cleared. Tor will choose.".into(),
    })
}

/// Parses an exclusion spec without touching anything: None for `none`.
pub fn parse_exclusions(spec: &str) -> Res<Vec<String>> {
    if spec.trim().is_empty() {
        return Err(Fail(
            "expected country codes, e.g. 'tormarchy exit --exclude de,us', or 'none'".into(),
        ));
    }
    if spec.trim() == "none" {
        return Ok(vec![]);
    }
    let mut codes = Vec::new();
    for part in spec.split(',') {
        let compact: String = part.chars().filter(|c| !c.is_whitespace()).collect();
        let cc = valid_code(&compact)
            .ok_or_else(|| Fail(format!("not a two-letter country code: {compact}")))?;
        if !codes.contains(&cc) {
            codes.push(cc);
        }
    }
    if codes.len() > 32 {
        return Err(Fail("too many countries; exclude at most 32".into()));
    }
    Ok(codes)
}

/// `exit --exclude de,us` / `--exclude none`. Unlike a pin it barely narrows
/// the pool, so it is the better tool for "anywhere but there".
pub fn exclude(host: &dyn Host, codes: &[String]) -> Res<String> {
    let paths = host.paths();
    if !paths.torrc_dir.is_dir() {
        return Err(Fail(format!(
            "{} is missing -- run setup first",
            paths.torrc_dir.display()
        )));
    }
    // Pinned to a country and excluding it leaves no exit at all.
    if let Some(pinned) = requested(paths) {
        if codes.contains(&pinned) {
            return Err(Fail(format!(
                "exits are pinned to {}, so it cannot also be excluded. Run 'tormarchy exit auto' first.",
                pinned.to_uppercase()
            )));
        }
    }
    if codes.is_empty() {
        sys::remove_file_quiet(&paths.torrc_exclude);
    } else {
        let list: Vec<String> = codes.iter().map(|c| format!("{{{c}}}")).collect();
        let conf = format!(
            "# Managed by tormarchy. Remove this file or run 'tormarchy exit --exclude none'.\nExcludeExitNodes {}\n",
            list.join(",")
        );
        sys::write_atomic(&paths.torrc_exclude, conf.as_bytes(), 0o644)
            .map_err(|e| Fail(e.to_string()))?;
    }
    reload(host)?;
    Ok(if codes.is_empty() {
        "No countries excluded. Tor may exit anywhere.".into()
    } else {
        format!(
            "Exits will never be in {}. New circuits follow the rule.",
            codes.join(" ").to_uppercase()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusions_are_lowercased_deduplicated_and_bounded() {
        assert_eq!(parse_exclusions("us, RU,us").unwrap(), ["us", "ru"]);
        assert_eq!(parse_exclusions("none").unwrap(), Vec::<String>::new());
        assert!(parse_exclusions("u5").is_err());
        assert!(parse_exclusions("").is_err());
        let many: Vec<String> = (0..33)
            .map(|i| format!("{}{}", (b'a' + i / 26) as char, (b'a' + i % 26) as char))
            .collect();
        assert!(parse_exclusions(&many.join(",")).is_err());
    }

    #[test]
    fn written_files_read_back() {
        let root = sys::create_temp_dir("tm-exit-").unwrap();
        let paths = Paths::under(&root);
        std::fs::create_dir_all(&paths.torrc_dir).unwrap();
        std::fs::write(
            &paths.torrc_exclude,
            "# x\nExcludeExitNodes {us},{RU},{zzz}\n",
        )
        .unwrap();
        assert_eq!(excluded(&paths), ["us", "ru"]);
        std::fs::write(&paths.torrc_exit, "ExitNodes {DE}\nStrictNodes 1\n").unwrap();
        assert_eq!(requested(&paths).as_deref(), Some("de"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
