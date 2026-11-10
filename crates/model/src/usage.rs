//! Token usage: a row for each call to a model (docs/data-model.md#个人资料和配置).

use sqlx::SqlitePool;

use crate::{Chosen, pricing};

/// The tokens one call took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    /// With the cached ones.
    pub input: i64,
    pub cached: i64,
    pub output: i64,
}

/// What a call was for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, sqlx::Type, ts_rs::TS)]
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

pub async fn record(
    db: &SqlitePool,
    chosen: &Chosen,
    purpose: Purpose,
    usage: Usage,
    now: i64,
) -> Result<(), sqlx::Error> {
    let off_peak = off_peak(chosen, now);
    // The configuration may have been deleted while the model was answering.
    sqlx::query!(
        "INSERT INTO usage (at, model_config_id, model, purpose, input_tokens, cached_tokens,
           output_tokens, off_peak)
         VALUES (?, (SELECT id FROM model_config WHERE id = ?), ?, ?, ?, ?, ?, ?)",
        now,
        chosen.id,
        chosen.model,
        purpose,
        usage.input,
        usage.cached,
        usage.output,
        off_peak,
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Usage is retained even when a claim is invalidated after the provider has answered.
pub async fn record_crawl(
    db: &SqlitePool,
    chosen: &Chosen,
    crawl_id: i64,
    usage: Usage,
    now: i64,
) -> Result<(), sqlx::Error> {
    let off_peak = off_peak(chosen, now);
    sqlx::query!(
        r#"INSERT INTO usage (at, model_config_id, model, purpose, crawl_id, input_tokens,
            cached_tokens, output_tokens, off_peak)
        VALUES (?, (SELECT id FROM model_config WHERE id = ?), ?, 'crawl',
            (SELECT id FROM crawl WHERE id = ?), ?, ?, ?, ?)"#,
        now,
        chosen.id,
        chosen.model,
        crawl_id,
        usage.input,
        usage.cached,
        usage.output,
        off_peak,
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Keep billed calls even if their email was deleted while the provider was answering.
pub async fn record_for_email(
    db: &SqlitePool,
    chosen: &Chosen,
    purpose: Purpose,
    email_id: Option<i64>,
    usage: Usage,
    now: i64,
) -> Result<(), sqlx::Error> {
    let off_peak = off_peak(chosen, now);
    sqlx::query!(
        "INSERT INTO usage (at, model_config_id, model, purpose, email_id,
           input_tokens, cached_tokens, output_tokens, off_peak)
         VALUES (?, (SELECT id FROM model_config WHERE id = ?), ?, ?,
           (SELECT id FROM email WHERE id = ?), ?, ?, ?, ?)",
        now,
        chosen.id,
        chosen.model,
        purpose,
        email_id,
        usage.input,
        usage.cached,
        usage.output,
        off_peak,
    )
    .execute(db)
    .await?;
    Ok(())
}
