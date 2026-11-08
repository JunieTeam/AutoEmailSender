//! The log files the desktop app keeps in `logs` under its data directory, read back to export
//! them and to send them with feedback.

use std::io;
use std::path::Path;

use chrono::{DateTime, Utc};

/// Files are named app.<date>.log, one for each day the app runs.
pub const PREFIX: &str = "app";
pub const SUFFIX: &str = "log";

/// The log files in `dir`, oldest first: each one's name and text.
pub fn files(dir: &Path) -> io::Result<Vec<(String, String)>> {
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
    let mut files = Vec::new();
    for name in names {
        match std::fs::read(dir.join(&name)) {
            // An app stopped partway through a line leaves a character cut short.
            Ok(bytes) => files.push((name, String::from_utf8_lossy(&bytes).into_owned())),
            // The oldest goes when a new day's file begins.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(files)
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
    for (_, text) in files(dir)? {
        for line in text.lines() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_cut_short_by_a_stop_leaves_the_rest_readable() {
        let dir = tempfile::tempdir().unwrap();
        // "导" with its last byte missing.
        let log = b"2026-11-07T08:00:00.000000Z  INFO app: started\n\
            2026-11-07T08:00:01.000000Z  INFO app: \xe5\xaf";
        std::fs::write(dir.path().join("app.2026-11-07.log"), log).unwrap();
        let texts: Vec<_> = events(dir.path())
            .unwrap()
            .into_iter()
            .map(|e| e.text)
            .collect();
        assert_eq!(
            texts,
            [
                "2026-11-07T08:00:00.000000Z  INFO app: started\n",
                "2026-11-07T08:00:01.000000Z  INFO app: \u{fffd}\n"
            ]
        );
    }
}
