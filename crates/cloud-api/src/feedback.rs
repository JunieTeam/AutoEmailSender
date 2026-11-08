//! Feedback the server passes on to the support mailbox without keeping it.

use serde::{Deserialize, Serialize};

/// The most characters a description may have once trimmed; it needs at least one.
pub const DESCRIPTION_CHARS: usize = 2000;
/// The most bytes of the log, as UTF-8.
pub const LOG_BYTES: usize = 2 * 1024 * 1024;
/// The most bytes of `version` and of `os`.
pub const FIELD_BYTES: usize = 100;
/// The most bytes of a request's body.
pub const REQUEST_BYTES: usize = LOG_BYTES + LOG_BYTES / 4;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Feedback {
    /// 32 lowercase hexadecimal digits, the same each time one feedback is tried again, so that
    /// support can tell the copies apart.
    pub id: String,
    pub description: String,
    pub version: String,
    /// The system, its version and the architecture.
    pub os: String,
    pub log: Option<String>,
}

/// Whether the server takes this description, as it was typed.
pub fn description_fits(description: &str) -> bool {
    (1..=DESCRIPTION_CHARS).contains(&description.trim().chars().count())
}
