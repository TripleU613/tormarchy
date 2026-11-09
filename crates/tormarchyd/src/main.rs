//! tormarchyd -- the panel's side of Tormarchy, as a child of the shell.
//!
//! The shell's plugin service runs this once for every monitor's widget and
//! speaks to it in JSON lines: commands in on stdin, the whole view out on
//! stdout whenever it changes. It holds one session to tor's control port,
//! subscribed to events, so the bar learns of a bootstrap step or a new circuit
//! when it happens instead of asking every few seconds. Everything the panel
//! shows is decided here; the QML only draws it.
//!
//! Nothing here runs as root. Routing changes go to the installed helper, which
//! asks polkit, exactly as a terminal user would.

mod daemon;

use std::io::{self, BufRead, Read};
use std::sync::mpsc;
use std::thread;

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("tormarchyd {}", tormarchy_core::VERSION);
        return;
    }
    let (tx, rx) = mpsc::channel();
    let input = tx.clone();
    thread::spawn(move || {
        // One command per line, each bounded: a line past 128 KiB is not one
        // this panel sent, and is dropped whole rather than parsed in part.
        let stdin = io::stdin();
        let mut reader = stdin.lock();
        loop {
            let mut line = Vec::new();
            let n = (&mut reader)
                .take(128 * 1024 + 1)
                .read_until(b'\n', &mut line);
            match n {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if line.len() > 128 * 1024 {
                        // Skip the rest of the oversized line.
                        let mut sink = Vec::new();
                        let _ = reader.read_until(b'\n', &mut sink);
                        continue;
                    }
                    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&line) {
                        if input.send(daemon::Ev::Input(v)).is_err() {
                            break;
                        }
                    }
                }
            }
        }
        let _ = input.send(daemon::Ev::InputClosed);
    });

    let ticker = tx.clone();
    thread::spawn(move || loop {
        thread::sleep(std::time::Duration::from_millis(500));
        if ticker.send(daemon::Ev::Tick).is_err() {
            break;
        }
    });

    daemon::Daemon::new(tx).run(rx);
}
