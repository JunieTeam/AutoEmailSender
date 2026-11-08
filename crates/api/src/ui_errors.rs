//! Defects of the UI's own, written to the log (docs/development.md#错误和日志).

use serde::Deserialize;
use ts_rs::TS;

use crate::{CallError, Context};

/// The most of a message or a stack kept, in characters.
const KEPT: usize = 2000;

#[derive(Debug, Deserialize, TS)]
pub struct UiErrorReport {
    pub message: String,
    pub stack: Option<String>,
    /// The route's template, such as `/mentors/$id`, which leaves out what was searched for.
    pub route: String,
}

pub async fn log_ui_error(_: &Context, input: UiErrorReport) -> Result<(), CallError> {
    tracing::error!(
        route = cut(&input.route),
        stack = input.stack.as_deref().map(cut),
        "ui error: {}",
        cut(&input.message)
    );
    Ok(())
}

fn cut(text: &str) -> &str {
    text.char_indices()
        .nth(KEPT)
        .map_or(text, |(end, _)| &text[..end])
}
