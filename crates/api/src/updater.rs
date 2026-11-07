//! Where desktop updates come from, and the update state returned by the application commands.

use serde::Serialize;
use ts_rs::TS;

/// The manifest of the latest GitHub Release, checked first.
pub const GITHUB_MANIFEST: &str =
    "https://github.com/JunieXD/AutoEmailSender/releases/latest/download/latest.json";

/// The update mirror, for when GitHub cannot be reached or is too slow: it serves `latest.json`,
/// and each version's files under `<version>/`.
pub const MIRROR: &str = "https://dl.auto-email-sender.cn/";

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    /// Unix milliseconds, or null when the release has no publication date.
    pub published_at: Option<i64>,
    pub notes: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UpdateStatus {
    Current,
    Available {
        update: UpdateInfo,
        error: Option<String>,
    },
    Downloading {
        update: UpdateInfo,
        downloaded: u64,
        total: Option<u64>,
    },
    Ready {
        update: UpdateInfo,
    },
}
