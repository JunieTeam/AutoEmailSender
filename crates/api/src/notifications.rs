//! The system's permission to show the app's notifications, returned by the application commands.

use serde::Serialize;
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum NotificationPermission {
    /// The system shows notifications without asking (Windows), or cannot be asked, as when the
    /// app does not run from its bundle.
    Unsupported,
    NotAsked,
    Allowed,
    Denied,
}
