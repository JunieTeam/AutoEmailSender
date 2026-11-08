//! Feedback sent to the support mailbox through the official server (docs/development.md#问题反馈).

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use chrono::{DateTime, Utc};
use cloud_api::feedback::{self as wire, LOG_BYTES};
use cloud_api::{ApiError, ErrorCode as ServerCode, path};
use failure::Failure;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{CallError, Context, Nothing};

/// The slowest the feedback may upload, in bytes a second, as an email may (crates/transport).
const UPLOAD_RATE: u64 = 32 * 1024;
/// How long the server may take once it has the feedback: waiting its turn and handing it over.
const PASSING_ON: Duration = Duration::from_secs(25);

/// The feedback the window shows. Only one at a time.
#[derive(Default)]
pub struct Feedback(Mutex<Option<Snapshot>>);

impl Feedback {
    fn lock(&self) -> MutexGuard<'_, Option<Snapshot>> {
        self.0
            .lock()
            .unwrap_or_else(|_| unreachable!("nothing panics while holding the lock"))
    }
}

/// The log as it was first shown, which every try to submit sends.
struct Snapshot {
    log: String,
    days: Option<LogDays>,
    /// The last try not known to have got through: one that failed, or whose answer was lost.
    tried: Option<Tried>,
}

/// A try to submit, which another with the same account, description and choice repeats.
struct Tried {
    id: String,
    email: String,
    description: String,
    with_log: bool,
}

/// The days of the oldest and newest events kept, as Unix milliseconds.
#[derive(Debug, Clone, Copy, Serialize, TS)]
pub struct LogDays {
    pub first: i64,
    pub last: i64,
}

#[derive(Debug, Serialize, TS)]
pub struct FeedbackPreview {
    /// The bytes of the log as sent.
    pub size: usize,
    /// None when nothing has been logged.
    pub days: Option<LogDays>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackInput {
    pub description: String,
    pub with_log: bool,
}

impl Snapshot {
    fn preview(&self) -> FeedbackPreview {
        FeedbackPreview {
            size: self.log.len(),
            days: self.days,
        }
    }
}

/// What the feedback dialog shows as it opens: the log of a try not known to have got through,
/// or else the log as it is now.
pub async fn preview_feedback(ctx: &Context, _: Nothing) -> Result<FeedbackPreview, CallError> {
    if let Some(snapshot) = &*ctx.feedback.lock()
        && snapshot.tried.is_some()
    {
        return Ok(snapshot.preview());
    }
    let dir = ctx.logs.clone();
    let snapshot = tokio::task::spawn_blocking(move || snapshot(&dir))
        .await
        .map_err(CallError::internal)?
        .map_err(|e| CallError::external(failure::file("日志未能读取", &e), e))?;
    let mut kept = ctx.feedback.lock();
    match &*kept {
        // A try that began meanwhile keeps the log it sent.
        Some(tried) if tried.tried.is_some() => Ok(tried.preview()),
        _ => {
            let preview = snapshot.preview();
            *kept = Some(snapshot);
            Ok(preview)
        }
    }
}

/// The whole log the preview stands for, scrubbed, as it would be sent.
pub async fn get_feedback_log(ctx: &Context, _: Nothing) -> Result<String, CallError> {
    match &*ctx.feedback.lock() {
        Some(snapshot) => Ok(snapshot.log.clone()),
        None => Err(CallError::internal(
            "the feedback log was asked for before a preview",
        )),
    }
}

pub async fn submit_feedback(ctx: &Context, input: FeedbackInput) -> Result<(), CallError> {
    if !wire::description_fits(&input.description) {
        return Err(CallError::invalid(format!(
            "请写下遇到的问题，最多 {} 字",
            wire::DESCRIPTION_CHARS
        )));
    }
    let login = account::saved_login(&ctx.db).await?;
    let description = input.description.trim().to_owned();
    let feedback = {
        let mut kept = ctx.feedback.lock();
        let snapshot = kept
            .as_mut()
            .ok_or_else(|| CallError::internal("feedback was submitted before a preview"))?;
        let id = match &snapshot.tried {
            Some(tried)
                if tried.email == login.email
                    && tried.description == description
                    && tried.with_log == input.with_log =>
            {
                tried.id.clone()
            }
            _ => {
                let id = uuid::Uuid::new_v4().simple().to_string();
                snapshot.tried = Some(Tried {
                    id: id.clone(),
                    email: login.email.clone(),
                    description: description.clone(),
                    with_log: input.with_log,
                });
                id
            }
        };
        wire::Feedback {
            id,
            description,
            version: env!("CARGO_PKG_VERSION").into(),
            os: os(),
            log: input.with_log.then(|| snapshot.log.clone()),
        }
    };
    let bytes = feedback.log.as_ref().map_or(0, String::len) as u64;
    let request = ctx
        .account
        .request(Method::POST, path::FEEDBACK)
        .json(&feedback)
        .timeout(Duration::from_secs(30) + Duration::from_secs(bytes / UPLOAD_RATE) + PASSING_ON);
    match ctx.account.send(&ctx.db, &login, request).await {
        Ok(_) => {
            tracing::info!(feedback = feedback.id, bytes, "feedback submitted");
            let mut kept = ctx.feedback.lock();
            if kept
                .as_ref()
                .and_then(|s| s.tried.as_ref())
                .is_some_and(|tried| tried.id == feedback.id)
            {
                *kept = None;
            }
            Ok(())
        }
        Err(account::Error::Server(ApiError {
            code: ServerCode::RateLimited,
            retry_after_seconds,
            ..
        })) => {
            let wait = match retry_after_seconds {
                Some(seconds @ 3600..) => format!("请在 {} 小时后再试", seconds.div_ceil(3600)),
                Some(seconds) => format!("请在 {} 分钟后再试", seconds.div_ceil(60).max(1)),
                None => "请稍后再试".into(),
            };
            Err(CallError::external(
                Failure::new(format!("反馈次数已达上限，{wait}")),
                format!("feedback {} rate limited", feedback.id),
            ))
        }
        Err(e) => Err(CallError::from(e).logging(format!("feedback {}", feedback.id))),
    }
}

/// The system, its version and the architecture, such as "Mac OS 15.4.1 aarch64".
fn os() -> String {
    let info = os_info::get();
    format!(
        "{} {} {}",
        info.os_type(),
        info.version(),
        std::env::consts::ARCH
    )
}

fn snapshot(dir: &Path) -> std::io::Result<Snapshot> {
    let events = crate::logs::events(dir)?
        .into_iter()
        .map(|e| (e.at, failure::scrub(&e.text)));
    let kept = newest(events, LOG_BYTES);
    let day = |at: Option<DateTime<Utc>>| at.map(|at| at.timestamp_millis());
    let days = day(kept.iter().find_map(|(at, _)| *at))
        .zip(day(kept.iter().rev().find_map(|(at, _)| *at)))
        .map(|(first, last)| LogDays { first, last });
    Ok(Snapshot {
        log: kept.into_iter().map(|(_, text)| text).collect(),
        days,
        tried: None,
    })
}

/// The newest of `events`, oldest first, that fit in `limit` bytes once written as a JSON string,
/// as the request carries the log: an older one is dropped whole.
fn newest<T>(
    events: impl DoubleEndedIterator<Item = (T, String)>,
    limit: usize,
) -> Vec<(T, String)> {
    // The quotes around the string.
    let mut size = 2;
    let mut kept: Vec<_> = events
        .rev()
        .take_while(|(_, text)| {
            size += serde_json::to_string(text)
                .unwrap_or_else(|_| unreachable!("a string is always written"))
                .len()
                - 2;
            size <= limit
        })
        .collect();
    kept.reverse();
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_oldest_events_go_whole_once_the_log_is_over_the_limit() {
        let events = ["a\n", "panic\n\"why\"\n", "c\n"].map(|text| ((), text.to_owned()));
        // The quotes, then each event with its newlines and quotes escaped.
        let all = 2 + 3 + 16 + 3;
        let kept = |limit| {
            newest(events.clone().into_iter(), limit)
                .into_iter()
                .map(|(_, text)| text)
                .collect::<String>()
        };
        assert_eq!(kept(all), "a\npanic\n\"why\"\nc\n");
        assert_eq!(kept(all - 1), "panic\n\"why\"\nc\n");
        assert_eq!(kept(2 + 3), "c\n");
        assert_eq!(kept(2 + 2), "");
    }
}
