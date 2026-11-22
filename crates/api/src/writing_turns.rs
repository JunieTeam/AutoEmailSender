//! Turns at the model for a composer's AI writing, which the user can cancel until the call goes
//! out (docs/design.md#联系).

use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex;

use model::Chosen;
use model::background::Priority;
use serde::Deserialize;
use tokio::sync::oneshot;
use ts_rs::TS;

use crate::{CallError, Context};

/// A composer's AI writing request.
#[derive(Debug, Deserialize, TS)]
pub struct Writing<T> {
    /// Chosen by the app's UI, so that `cancelWriting` can withdraw the call before it reaches the
    /// model; agents leave it out.
    #[serde(default)]
    #[ts(optional)]
    pub request: Option<String>,
    #[serde(flatten)]
    pub edit: T,
}

#[derive(Debug, Deserialize, TS)]
pub struct WritingRequest {
    pub request: String,
}

/// The UI's calls, by request, until each ends. The sender is there until `Turn::send` or
/// `cancel_writing` takes it, whichever comes first; dropping it wakes a call still waiting for a
/// turn.
#[derive(Default)]
pub struct Writings(Mutex<HashMap<String, Option<oneshot::Sender<()>>>>);
impl Writings {
    fn calls(&self) -> std::sync::MutexGuard<'_, HashMap<String, Option<oneshot::Sender<()>>>> {
        self.0
            .lock()
            .unwrap_or_else(|_| unreachable!("nothing panics while holding the lock"))
    }
}

/// A request in `Writings`, taken out however its call ends.
struct Listed<'a> {
    writings: &'a Writings,
    request: String,
}
impl Drop for Listed<'_> {
    fn drop(&mut self) {
        self.writings.calls().remove(&self.request);
    }
}

/// A turn at the model; until `send`, the UI can still withdraw the call. It keeps the request
/// taken until the call ends.
pub(crate) struct Turn<'a> {
    model: model::background::Turn,
    listed: Option<Listed<'a>>,
}
impl Turn<'_> {
    /// Right before the call goes to the model: withdrawn by now, it does not go; from here on it
    /// goes whatever the UI does.
    pub(crate) fn send(&self) -> Result<&model::background::Turn, CallError> {
        let Some(listed) = &self.listed else {
            return Ok(&self.model);
        };
        let mut calls = listed.writings.calls();
        match calls.get_mut(&listed.request).and_then(Option::take) {
            Some(_) => Ok(&self.model),
            None => Err(canceled()),
        }
    }
}

fn canceled() -> CallError {
    CallError::invalid("这个请求已经取消".into())
}

impl Context {
    /// A turn at the model of the configuration `read` gives, as it is once the turn comes, unless
    /// the UI cancels `request` first.
    pub(crate) async fn writing_turn<E, F>(
        &self,
        request: Option<String>,
        read: impl Fn() -> F,
    ) -> Result<(Turn<'_>, Chosen), CallError>
    where
        F: Future<Output = Result<Option<Chosen>, E>>,
        E: Into<CallError>,
    {
        let take = async {
            self.turns
                .take_current(Priority::Interactive, read)
                .await
                .map_err(Into::<CallError>::into)?
                .ok_or_else(|| CallError::from(writing::Error::NoModel))
        };
        let Some(request) = request else {
            let (model, chosen) = take.await?;
            return Ok((
                Turn {
                    model,
                    listed: None,
                },
                chosen,
            ));
        };
        let (cancel, canceled_early) = oneshot::channel();
        {
            let mut calls = self.writings.calls();
            if calls.contains_key(&request) {
                return Err(CallError::bad_input("这个请求编号已经在用"));
            }
            calls.insert(request.clone(), Some(cancel));
        }
        let listed = Listed {
            writings: &self.writings,
            request,
        };
        tokio::select! {
            taken = take => {
                let (model, chosen) = taken?;
                Ok((Turn { model, listed: Some(listed) }, chosen))
            }
            _ = canceled_early => Err(canceled()),
        }
    }
}

/// Withdraws a call not sent to the model yet; one already sent goes on, and its usage is
/// recorded.
pub async fn cancel_writing(ctx: &Context, input: WritingRequest) -> Result<(), CallError> {
    if let Some(call) = ctx.writings.calls().get_mut(&input.request) {
        call.take();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::*;
    use std::{
        future::{Future, poll_fn},
        pin::Pin,
        task::Poll,
    };
    use test_fixtures::provider;

    #[sqlx::test(migrator = "db::MIGRATOR")]
    async fn a_letter_canceled_before_it_goes_out_never_reaches_the_model(db: SqlitePool) {
        let mentor = mentor::create(
            &db,
            &mentor::MentorFields {
                name: "王伟".into(),
                ..Default::default()
            },
            0,
        )
        .await
        .unwrap();
        let ctx = Context::new(db, net::Net::new(vec![]).unwrap(), std::path::Path::new(""));
        let (address, received) =
            provider::scripted(vec![("<subject>申请</subject><body>信</body>", "stop")])
                .await
                .unwrap();
        let fields = model::ModelForm {
            name: String::new(),
            provider: model::providers::CUSTOM.into(),
            base_url: address.clone(),
            model: "m".into(),
            region: None,
            currency: None,
            prices: model::pricing::Prices::default(),
            off_peak: model::pricing::Prices::default(),
        }
        .check()
        .unwrap();
        let id = model::create(&ctx.db, &fields, "", 0).await.unwrap();
        assert!(model::agree(&ctx.db, id, &fields.source).await.unwrap());
        let chosen = model::chosen(&ctx.db, model::Feature::Writing)
            .await
            .unwrap()
            .unwrap();
        let take = || {
            ctx.turns.take(
                &chosen.access,
                &chosen.model,
                model::background::Priority::Background,
            )
        };
        let mut turns = Vec::new();
        for _ in 0..model::background::CONCURRENCY {
            turns.push(take().await);
        }
        let write = |request: &str| {
            call(
                &ctx,
                "writeLetter",
                serde_json::json!({
                    "mentorId": mentor, "subject": "", "bodyMd": "", "attachments": [],
                    "request": request
                }),
            )
        };
        let cancel = |request: &str| {
            call(
                &ctx,
                "cancelWriting",
                serde_json::json!({ "request": request }),
            )
        };
        async fn waiting(writing: Pin<&mut impl Future>) {
            let mut writing = writing;
            poll_fn(|cx| {
                assert!(writing.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
        }

        // Canceled while it waits for a turn, it keeps its request until it ends.
        let mut first = std::pin::pin!(write("r1"));
        waiting(first.as_mut()).await;
        cancel("r1").await.unwrap();
        let mut taken = std::pin::pin!(write("r1"));
        poll_fn(|cx| {
            let Poll::Ready(Err(e)) = taken.as_mut().poll(cx) else {
                panic!("a request still taken was accepted again");
            };
            assert!(matches!(e.code, ErrorCode::BadInput));
            Poll::Ready(())
        })
        .await;
        assert!(first.await.is_err());

        // Dropped while it waits, it leaves nothing behind.
        let mut dropped = Box::pin(write("r2"));
        waiting(dropped.as_mut()).await;
        drop(dropped);
        let mut again = std::pin::pin!(write("r2"));
        waiting(again.as_mut()).await;
        cancel("r2").await.unwrap();
        assert!(again.await.is_err());

        // Canceled while it reads what it needs, with turns free.
        let mut held = Vec::new();
        for _ in 0..ctx.db.options().get_max_connections() {
            held.push(ctx.db.acquire().await.unwrap());
        }
        drop(turns);
        let mut last = std::pin::pin!(write("r3"));
        waiting(last.as_mut()).await;
        cancel("r3").await.unwrap();
        drop(held);
        assert!(last.await.is_err());

        // Canceled once it holds its turn, it does not go out when it would.
        let (turn, _) = ctx
            .writing_turn(Some("r4".into()), || {
                model::chosen(&ctx.db, model::Feature::Writing)
            })
            .await
            .unwrap();
        cancel("r4").await.unwrap();
        assert!(turn.send().is_err());
        drop(turn);

        assert!(received.lock().unwrap().is_empty());
        // None of them kept a turn.
        let mut free = Vec::new();
        for _ in 0..model::background::CONCURRENCY {
            let mut turn = std::pin::pin!(take());
            free.push(
                poll_fn(|cx| match turn.as_mut().poll(cx) {
                    Poll::Ready(turn) => Poll::Ready(turn),
                    Poll::Pending => panic!("a call kept its turn"),
                })
                .await,
            );
        }
    }

    /// A template task with a draft, and a model to write letters with: the context, the task, the
    /// draft and what the model receives.
    async fn task_draft(db: SqlitePool) -> (Context, i64, i64, provider::Received) {
        let mentor = mentor::create(
            &db,
            &mentor::MentorFields {
                name: "王伟".into(),
                emails: vec!["wang@example.com".into()],
                ..Default::default()
            },
            0,
        )
        .await
        .unwrap();
        let template = template::create(
            &db,
            &template::TemplateForm {
                name: "首封".into(),
                subject: "申请".into(),
                body_md: "{导师姓名}老师：您好！".into(),
            }
            .check()
            .unwrap(),
            0,
        )
        .await
        .unwrap();
        let task = task::create(
            &db,
            &task::NewTask {
                name: "首封".into(),
                kind: task::Kind::First,
                writer: task::Writer::Template,
                ids: vec![mentor],
                schedule: email::schedule::Schedule::Now,
                template_id: Some(template),
                template_source_id: None,
                source_task_id: None,
                model_config_id: None,
                references: vec![],
                attachments: vec![],
            },
            0,
        )
        .await
        .unwrap();
        let draft: i64 = sqlx::query_scalar("SELECT id FROM email WHERE task_id = ?")
            .bind(task)
            .fetch_one(&db)
            .await
            .unwrap();
        let ctx = Context::new(db, net::Net::new(vec![]).unwrap(), std::path::Path::new(""));
        let (address, received) =
            provider::scripted(vec![("<subject>申请</subject><body>信</body>", "stop")])
                .await
                .unwrap();
        let fields = model::ModelForm {
            name: String::new(),
            provider: model::providers::CUSTOM.into(),
            base_url: address,
            model: "m".into(),
            region: None,
            currency: None,
            prices: model::pricing::Prices::default(),
            off_peak: model::pricing::Prices::default(),
        }
        .check()
        .unwrap();
        let id = model::create(&ctx.db, &fields, "", 0).await.unwrap();
        assert!(model::agree(&ctx.db, id, &fields.source).await.unwrap());
        (ctx, task, draft, received)
    }

    #[sqlx::test(migrator = "db::MIGRATOR")]
    async fn a_task_letter_canceled_while_its_task_is_read_never_reaches_the_model(db: SqlitePool) {
        let (ctx, task, draft, received) = task_draft(db).await;
        // Before it knows which model the task writes with, the UI can already withdraw it.
        let mut held = Vec::new();
        for _ in 0..ctx.db.options().get_max_connections() {
            held.push(ctx.db.acquire().await.unwrap());
        }
        let mut writing = std::pin::pin!(call(
            &ctx,
            "writeTaskDraft",
            serde_json::json!({
                "taskId": task, "id": draft, "subject": "", "bodyMd": "", "attachments": [],
                "request": "t"
            }),
        ));
        poll_fn(|cx| {
            assert!(writing.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        call(&ctx, "cancelWriting", serde_json::json!({ "request": "t" }))
            .await
            .unwrap();
        drop(held);
        assert!(writing.await.is_err());
        assert!(received.lock().unwrap().is_empty());
    }

    #[sqlx::test(migrator = "db::MIGRATOR")]
    async fn a_task_deleted_while_its_letter_waits_says_the_letter_is_gone(db: SqlitePool) {
        let (ctx, task, draft, received) = task_draft(db).await;
        let chosen = model::chosen(&ctx.db, model::Feature::Writing)
            .await
            .unwrap()
            .unwrap();
        let mut held = Vec::new();
        for _ in 0..model::background::CONCURRENCY {
            held.push(
                ctx.turns
                    .take(
                        &chosen.access,
                        &chosen.model,
                        model::background::Priority::Background,
                    )
                    .await,
            );
        }
        let mut writing = std::pin::pin!(call(
            &ctx,
            "writeTaskDraft",
            serde_json::json!({
                "taskId": task, "id": draft, "subject": "", "bodyMd": "", "attachments": []
            }),
        ));
        poll_fn(|cx| {
            assert!(writing.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        task::delete(&ctx.db, task, 0).await.unwrap();
        drop(held);
        let error = writing.await.unwrap_err();
        assert!(matches!(error.code, ErrorCode::NotFound));
        assert_eq!(error.message, "这封邮件已被删除");
        assert!(received.lock().unwrap().is_empty());
    }

    #[sqlx::test(migrator = "db::MIGRATOR")]
    async fn queued_letter_uses_the_model_address_saved_before_its_turn(db: SqlitePool) {
        let mentor = mentor::create(
            &db,
            &mentor::MentorFields {
                name: "王伟".into(),
                ..Default::default()
            },
            0,
        )
        .await
        .unwrap();
        let ctx = Context::new(db, net::Net::new(vec![]).unwrap(), std::path::Path::new(""));
        let (old_address, old_received) = provider::scripted(vec![(
            "<subject>申请</subject><body>旧地址的信</body>",
            "stop",
        )])
        .await
        .unwrap();
        let (new_address, new_received) = provider::scripted(vec![(
            "<subject>申请</subject><body>新地址的信</body>",
            "stop",
        )])
        .await
        .unwrap();
        let form = |address: &str| model::ModelForm {
            name: String::new(),
            provider: model::providers::CUSTOM.into(),
            base_url: address.into(),
            model: "m".into(),
            region: None,
            currency: None,
            prices: model::pricing::Prices::default(),
            off_peak: model::pricing::Prices::default(),
        };
        let id = model::create(&ctx.db, &form(&old_address).check().unwrap(), "", 0)
            .await
            .unwrap();
        assert!(
            model::agree(&ctx.db, id, &form(&old_address).check().unwrap().source)
                .await
                .unwrap()
        );

        let old = model::chosen(&ctx.db, model::Feature::Writing)
            .await
            .unwrap()
            .unwrap();
        let mut turns = Vec::new();
        for _ in 0..model::background::CONCURRENCY {
            turns.push(
                ctx.turns
                    .take(
                        &old.access,
                        &old.model,
                        model::background::Priority::Background,
                    )
                    .await,
            );
        }
        let mut writing = std::pin::pin!(call(
            &ctx,
            "writeLetter",
            serde_json::json!({
                "mentorId": mentor, "subject": "", "bodyMd": "", "attachments": []
            }),
        ));
        poll_fn(|cx| {
            assert!(writing.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;

        update_model_config(
            &ctx,
            ModelConfigEdit {
                id,
                form: form(&new_address),
                api_key: String::new(),
            },
        )
        .await
        .unwrap();
        agree_model(
            &ctx,
            ModelAgreement {
                id,
                provider: "custom".into(),
                region: None,
                base_url: Some(new_address),
            },
        )
        .await
        .unwrap();
        // Whether it read the configuration before the edit or not, it calls the address saved
        // by the time its turn comes.
        drop(turns);
        let letter = writing.await.unwrap();

        assert!(
            old_received.lock().unwrap().is_empty(),
            "queued request reached the old model address"
        );
        assert_eq!(new_received.lock().unwrap().len(), 1);
        assert_eq!(
            letter,
            serde_json::json!({"subject": "申请", "bodyMd": "新地址的信"})
        );
    }

    #[sqlx::test(migrator = "db::MIGRATOR")]
    async fn first_follow_up_and_reply_checks_report_student_decisions(db: SqlitePool) {
        let mentor = mentor::create(
            &db,
            &mentor::MentorFields {
                name: "王伟".into(),
                ..Default::default()
            },
            now(),
        )
        .await
        .unwrap();
        let ctx = Context::new(db, net::Net::new(vec![]).unwrap(), std::path::Path::new(""));
        // Follow-ups share checkReplyDraft with replies; all entry points reach check::letter.
        for op in ["checkFirstDraft", "checkReplyDraft"] {
            let problems = call(
                &ctx,
                op,
                serde_json::json!({
                    "mentorId": mentor, "subject": "申请", "replyTo": "<target@example.edu>",
                    "bodyMd": "王老师：时间为【请补充：时间】。", "attachments": []
                }),
            )
            .await
            .unwrap();
            assert_eq!(
                problems,
                serde_json::json!([{
                    "kind": "placeholder", "name": "【请补充：时间】"
                }])
            );
        }
    }
}
