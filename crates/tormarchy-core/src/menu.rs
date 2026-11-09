//! Tor in the Omarchy menu.
//!
//! Omarchy has no way for a plugin to add menu rows of its own: the menu reads
//! its shipped defaults and the user's extension file, nothing else. So Tor's
//! rows go in that file, as a block between two marker comments this owns --
//! written by `add`, rewritten in place by a later add, taken out by `remove`.
//! Everything outside the markers is left exactly as it was. Always written as
//! the user, never as root.

use std::fs;
use std::path::{Path, PathBuf};

use crate::sys;
use crate::{Fail, Res};

pub const BEGIN: &str =
    "// >>> tormarchy: written by 'tormarchy menu add'. Edits between these lines are replaced.";
const BEGIN_PREFIX: &str = "// >>> tormarchy";
pub const END: &str = "// <<< tormarchy";

/// On: rules applied, or browser-only with tor up. The menu runs every check
/// each time it opens, so this is a file test before it is anything slower.
const ON: &str = "[ -e /run/tormarchy/active ] || { grep -qx socks /run/tormarchy/mode 2>/dev/null && systemctl is-active -q tor; }";

/// The menu reads $HOME/.config whatever XDG_CONFIG_HOME says, so this does too.
pub fn file(home: &Path) -> PathBuf {
    home.join(".config/omarchy/extensions/omarchy-menu.jsonc")
}

fn json(s: &str) -> String {
    serde_json::to_string(s).expect("strings serialise")
}

pub fn block() -> String {
    let row = |key: &str, fields: &[(&str, &str)]| {
        let body: Vec<String> = fields
            .iter()
            .map(|(k, v)| format!("{}:{}", json(k), json(v)))
            .collect();
        format!("  {}: {{{}}},\n", json(key), body.join(","))
    };
    let not_on = format!("! {{ {ON}; }}");
    let mut s = format!("  {BEGIN}\n");
    s += &row(
        "trigger.toggle.tor",
        &[
            ("icon", "󰕥"),
            ("label", "Tor"),
            ("checked", ON),
            ("action", "omarchy-shell tripleu.tor toggleTor"),
        ],
    );
    s += &row("trigger.tor", &[("icon", "󰕥"), ("label", "Tor")]);
    s += &row(
        "trigger.tor.connect",
        &[
            ("icon", "󰕥"),
            ("label", "Connect"),
            ("when", &not_on),
            ("action", "omarchy-shell tripleu.tor connect"),
        ],
    );
    s += &row(
        "trigger.tor.disconnect",
        &[
            ("icon", "󰦞"),
            ("label", "Disconnect"),
            ("when", ON),
            ("action", "omarchy-shell tripleu.tor disconnect"),
        ],
    );
    s += &row(
        "trigger.tor.new-circuit",
        &[
            ("icon", "󰑐"),
            ("label", "New Circuit"),
            ("when", ON),
            ("action", "omarchy-shell tripleu.tor newCircuit"),
        ],
    );
    s += &row(
        "trigger.tor.panel",
        &[
            ("icon", "󰒓"),
            ("label", "Panel"),
            ("action", "omarchy-shell tripleu.tor open"),
        ],
    );
    s += &row(
        "trigger.tor.leak-check",
        &[
            ("icon", "󰄬"),
            ("label", "Leak Check"),
            ("action", "omarchy-shell tripleu.tor checkLeaks"),
        ],
    );
    s += &row(
        "trigger.tor.bridges",
        &[
            ("icon", "󰘘"),
            ("label", "Bridges"),
            ("action", "omarchy-shell tripleu.tor bridges"),
        ],
    );
    s += &format!("  {END}\n");
    s
}

/// The text without our block.
pub fn without_block(text: &str) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        if !skipping && t.starts_with(BEGIN_PREFIX) {
            skipping = true;
            continue;
        }
        if skipping {
            if t.starts_with(END) {
                skipping = false;
            }
            continue;
        }
        out.push_str(line);
    }
    out
}

/// Our block right after the opening brace: entries end in commas, which the
/// menu's parser drops before a closing brace, so this is valid whatever
/// follows. None when there is no opening brace.
pub fn with_block(text: &str) -> Option<String> {
    let mut out = String::new();
    let mut done = false;
    for line in text.split_inclusive('\n') {
        if !done && !line.trim_start().starts_with("//") {
            if let Some(i) = line.find('{') {
                out.push_str(&line[..=i]);
                out.push('\n');
                out.push_str(&block());
                let rest = &line[i + 1..];
                if rest.trim().is_empty() {
                    // The line's own newline went with the brace above.
                } else {
                    out.push_str(rest);
                    if !rest.ends_with('\n') {
                        out.push('\n');
                    }
                }
                done = true;
                continue;
            }
        }
        out.push_str(line);
    }
    done.then_some(out)
}

/// Parsed the way the menu parses it: whole-line comments and trailing commas
/// stripped, then JSON, and it must be an object.
pub fn parses(text: &str) -> bool {
    let mut stripped = String::new();
    for line in text.lines() {
        if !line.trim_start().starts_with("//") {
            stripped.push_str(line);
            stripped.push('\n');
        }
    }
    let mut cleaned = String::with_capacity(stripped.len());
    let chars: Vec<char> = stripped.chars().collect();
    let mut in_string = false;
    let mut escaped = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            cleaned.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            cleaned.push(c);
        } else if c == ',' {
            let next = chars[i + 1..].iter().find(|c| !c.is_whitespace());
            if !matches!(next, Some('}') | Some(']')) {
                cleaned.push(c);
            }
        } else {
            cleaned.push(c);
        }
        i += 1;
    }
    if cleaned.trim().is_empty() {
        return true;
    }
    matches!(
        serde_json::from_str::<serde_json::Value>(&cleaned),
        Ok(serde_json::Value::Object(_))
    )
}

#[derive(Clone, Copy, PartialEq)]
pub enum Verb {
    Add,
    Remove,
}

/// Rewrites the user's file. Through a dotfiles symlink to the file it points
/// at, so the link survives; refuses to touch a file that does not parse now,
/// or would not after.
pub fn write(home: &Path, verb: Verb) -> Res<String> {
    let file = file(home);
    let target = fs::canonicalize(&file).unwrap_or_else(|_| file.clone());
    let existing = match fs::read_to_string(&target) {
        Ok(s) => Some(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(Fail(format!("could not read {}: {e}", file.display()))),
    };
    if verb == Verb::Remove && existing.is_none() {
        return Ok("No Tormarchy entries in the Omarchy menu.".into());
    }
    if let Some(text) = &existing {
        if !parses(text) {
            return Err(Fail(format!(
                "{} does not parse as it is; fix it first, nothing was changed",
                file.display()
            )));
        }
    }
    let next = match (&existing, verb) {
        (None, Verb::Add) => format!("{{\n{}}}\n", block()),
        (Some(t), Verb::Add) if t.trim().is_empty() => format!("{{\n{}}}\n", block()),
        (Some(t), Verb::Add) => with_block(&without_block(t)).ok_or_else(|| {
            Fail(format!(
                "{} has no opening brace; fix it first, nothing was changed",
                file.display()
            ))
        })?,
        (Some(t), Verb::Remove) => without_block(t),
        (None, Verb::Remove) => unreachable!(),
    };
    if !parses(&next) {
        return Err(Fail(
            "the menu would not parse after the change, so nothing was changed".into(),
        ));
    }
    let dir = target.parent().ok_or("no directory")?;
    fs::create_dir_all(dir)
        .map_err(|e| Fail(format!("could not create {}: {e}", dir.display())))?;
    let mode = fs::metadata(&target)
        .map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o7777)
        .unwrap_or(0o644);
    sys::write_atomic(&target, next.as_bytes(), mode)
        .map_err(|e| Fail(format!("could not replace {}: {e}", file.display())))?;
    Ok(match verb {
        Verb::Add => {
            "Tor is in the Omarchy menu: Trigger > Tor, and Trigger > Toggle > Tor.".into()
        }
        Verb::Remove => "Removed Tormarchy's entries from the Omarchy menu.".into(),
    })
}

pub fn present(home: &Path) -> bool {
    fs::read_to_string(file(home))
        .map(|t| t.contains(BEGIN_PREFIX))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> PathBuf {
        sys::create_temp_dir("tm-menu-").unwrap()
    }

    #[test]
    fn add_into_nothing_makes_a_valid_file() {
        let h = home();
        write(&h, Verb::Add).unwrap();
        let t = fs::read_to_string(file(&h)).unwrap();
        assert!(parses(&t));
        assert!(t.contains("\"trigger.toggle.tor\""));
        fs::remove_dir_all(h).unwrap();
    }

    #[test]
    fn add_keeps_everything_else_and_is_idempotent() {
        let h = home();
        fs::create_dir_all(file(&h).parent().unwrap()).unwrap();
        let original = "// mine\n{\n  \"x.y\": {\"label\": \"Mine\"},\n  // a note\n  \"z\": {\"label\": \"Z\"}\n}\n";
        fs::write(file(&h), original).unwrap();
        write(&h, Verb::Add).unwrap();
        let once = fs::read_to_string(file(&h)).unwrap();
        write(&h, Verb::Add).unwrap();
        assert_eq!(
            fs::read_to_string(file(&h)).unwrap(),
            once,
            "a second add rewrites in place"
        );
        assert!(once.contains("\"x.y\"") && once.contains("// a note"));
        write(&h, Verb::Remove).unwrap();
        assert_eq!(
            fs::read_to_string(file(&h)).unwrap(),
            original,
            "remove puts it back exactly"
        );
        fs::remove_dir_all(h).unwrap();
    }

    #[test]
    fn a_broken_file_is_left_alone() {
        let h = home();
        fs::create_dir_all(file(&h).parent().unwrap()).unwrap();
        fs::write(file(&h), "{ \"a\": ").unwrap();
        assert!(write(&h, Verb::Add).is_err());
        assert_eq!(fs::read_to_string(file(&h)).unwrap(), "{ \"a\": ");
        fs::remove_dir_all(h).unwrap();
    }

    #[test]
    fn a_symlinked_file_stays_a_symlink() {
        let h = home();
        let real = h.join("dotfiles/menu.jsonc");
        fs::create_dir_all(real.parent().unwrap()).unwrap();
        fs::write(&real, "{}\n").unwrap();
        fs::create_dir_all(file(&h).parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&real, file(&h)).unwrap();
        write(&h, Verb::Add).unwrap();
        assert!(fs::symlink_metadata(file(&h))
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(fs::read_to_string(&real).unwrap().contains("trigger.tor"));
        fs::remove_dir_all(h).unwrap();
    }

    #[test]
    fn trailing_commas_and_comments_parse_like_the_menu_does() {
        assert!(parses("// c\n{\n \"a\": {\"b\": \"x, }\"},\n}\n"));
        assert!(!parses("[1, 2]"));
    }
}
