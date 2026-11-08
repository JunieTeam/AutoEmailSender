//! The log files the desktop app keeps in `logs` under its data directory, read back to export
//! them and to send them with feedback.

use std::io;
use std::path::Path;

use chrono::{DateTime, Utc};

/// Files are named app.<date>.log, one for each day the app runs.
pub const PREFIX: &str = "app";
pub const SUFFIX: &str = "log";

/// The names of the log files in `dir`, oldest first.
pub fn files(dir: &Path) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        // Finder and others leave their own files here, such as .DS_Store.
        if name.starts_with(PREFIX) && name.ends_with(SUFFIX) {
            names.push(name);
        }
    }
    // Daily file names sort by date.
    names.sort();
    Ok(names)
}

/// One logged event, its lines each ending in a newline.
pub struct Event {
    /// When it was logged, which its first line starts with; none for lines before any event.
    pub at: Option<DateTime<Utc>>,
    pub text: String,
}

/// Every event logged in `dir`, oldest first.
pub fn events(dir: &Path) -> io::Result<Vec<Event>> {
    let mut events: Vec<Event> = Vec::new();
    for name in files(dir)? {
        for line in std::fs::read_to_string(dir.join(name))?.lines() {
            let at = line
                .split_whitespace()
                .next()
                .and_then(|first| DateTime::parse_from_rfc3339(first).ok());
            match (at, events.last_mut()) {
                // An event's later lines, such as a panic's message, have no time of their own.
                (None, Some(event)) => {
                    event.text.push_str(line);
                    event.text.push('\n');
                }
                (at, _) => events.push(Event {
                    at: at.map(|at| at.to_utc()),
                    text: format!("{line}\n"),
                }),
            }
        }
    }
    Ok(events)
}
