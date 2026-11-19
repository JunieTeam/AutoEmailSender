//! Token usage: a row for each call to a model (docs/data-model.md#个人资料和配置).

use failure::Failure;
use sqlx::{SqlitePool, types::Json};

use crate::{Answer, Chosen, ModelError, pricing};

/// The tokens one call took.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
pub struct Usage {
    /// With the cached ones.
    pub input: i64,
    pub cached: i64,
    pub output: i64,
}

/// What a call was for.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, sqlx::Type, ts_rs::TS,
)]
#[sqlx(rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
#[ts(rename = "UsagePurpose")]
pub enum Purpose {
    Crawl,
    Match,
    Draft,
    Rewrite,
    FactCheck,
    ReplyOutcome,
    ReplyTodos,
}

/// Whether the call fell in its provider's off-peak hours, which charge its off-peak prices. Kept
/// with the call, as the hour it was made does not change.
fn off_peak(chosen: &Chosen, at: i64) -> bool {
    pricing::off_peak(
        chosen.access.provider.and_then(|p| p.peak_hours.as_ref()),
        at,
    )
}

/// Whom or what a call was for: a crawl, or a mentor with the email it wrote, checked or read the
/// reply to, once there is one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct About {
    pub crawl: Option<i64>,
    pub mentor: Option<i64>,
    pub email: Option<i64>,
}

/// What came of a call, as `ask` came back from it.
#[derive(Debug, Clone)]
pub enum Outcome {
    /// When the request was sent, and what it took if the provider said.
    Answered { at: i64, usage: Option<Usage> },
    /// When it failed, and why.
    Failed { at: i64, failure: Failure },
}

impl Outcome {
    pub fn of(result: &Result<Answer, ModelError>) -> Self {
        match result {
            Ok(answer) => Outcome::Answered {
                at: answer.sent_at,
                usage: answer.usage,
            },
            Err(e) => Outcome::Failed {
                at: crate::client::now(),
                failure: e.failure(),
            },
        }
    }
}

/// Records a call. The configuration, crawl, mentor or email may have been deleted while the model
/// was answering, and is then left out; a mentor merged away meanwhile gives way to whoever has the
/// email now, the one kept. Not the email's mentor first: a reply in the thread of a letter to one
/// mentor may be another's.
pub async fn record(
    db: &SqlitePool,
    chosen: &Chosen,
    purpose: Purpose,
    about: About,
    outcome: Outcome,
) -> Result<(), sqlx::Error> {
    let (at, usage, failure) = match outcome {
        Outcome::Answered { at, usage } => (at, usage, None),
        Outcome::Failed { at, failure } => (at, None, Some(Json(failure))),
    };
    let off_peak = off_peak(chosen, at);
    sqlx::query!(
        "INSERT INTO usage (at, model_config_id, model, purpose, crawl_id, mentor_id, email_id,
           input_tokens, cached_tokens, output_tokens, off_peak, error)
         VALUES (?1, (SELECT id FROM model_config WHERE id = ?2), ?3, ?4,
           (SELECT id FROM crawl WHERE id = ?5),
           coalesce((SELECT id FROM mentor WHERE id = ?6), (SELECT mentor_id FROM email WHERE id = ?7)),
           (SELECT id FROM email WHERE id = ?7), ?8, ?9, ?10, ?11, ?12)",
        at,
        chosen.id,
        chosen.model,
        purpose,
        about.crawl,
        about.mentor,
        about.email,
        usage.map(|u| u.input),
        usage.map(|u| u.cached),
        usage.map(|u| u.output),
        off_peak,
        failure,
    )
    .execute(db)
    .await?;
    Ok(())
}
