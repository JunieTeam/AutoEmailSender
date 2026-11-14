use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(rename_all = "snake_case")]
pub enum PageKind {
    Entry,
    List,
    Profile,
    EnrichProfile,
    EnrichChild,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(rename_all = "snake_case")]
pub enum PageMethod {
    Http,
    Webview,
}

/// A control clicked once the page has opened, by its raw label; the serialized steps identify
/// the page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageStep {
    /// The page of the list it turns to; none for a control showing another part of the page, such
    /// as one category of a list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    pub control: String,
}

impl PageStep {
    /// The page of the list that `steps` end on.
    pub fn page_of(steps: &[PageStep]) -> u32 {
        steps.iter().rev().find_map(|s| s.page).unwrap_or(1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(rename_all = "snake_case")]
pub enum PageStatus {
    Success,
    Failed,
    NeedsBrowser,
}
