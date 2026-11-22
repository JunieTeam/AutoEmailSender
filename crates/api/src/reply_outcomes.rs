//! The outcome of a mentor's reply, set by the student or read by AI.

use crate::*;

#[derive(Debug, Deserialize, TS)]
pub struct AiReplyOutcome {
    pub enabled: bool,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReplyMessage {
    pub message_id: i64,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReplyOutcomeEdit {
    pub message_id: i64,
    pub outcome: email::outcome::ReplyOutcome,
}

pub async fn get_ai_reply_outcome(ctx: &Context, _: Nothing) -> Result<bool, CallError> {
    Ok(email::outcome::enabled(&ctx.db).await?)
}
pub async fn set_ai_reply_outcome(ctx: &Context, input: AiReplyOutcome) -> Result<(), CallError> {
    email::outcome::set_enabled(&ctx.db, input.enabled).await?;
    ctx.reply_processor.restart();
    Ok(())
}
pub async fn set_reply_outcome(ctx: &Context, input: ReplyOutcomeEdit) -> Result<(), CallError> {
    if !email::outcome::set(&ctx.db, input.message_id, input.outcome).await? {
        return Err(CallError::invalid(
            "只能标记导师的回信，请重新选择回信".into(),
        ));
    }
    ctx.reply_processor.wake();
    Ok(())
}
pub async fn retry_reply_outcome(ctx: &Context, input: ReplyMessage) -> Result<(), CallError> {
    require_applied(
        email::outcome::retry(&ctx.db, input.message_id).await?,
        "这封回信无可重试的标记",
    )?;
    ctx.reply_processor.wake();
    Ok(())
}
pub async fn reply_processing(
    ctx: &Context,
    _: Nothing,
) -> Result<email::outcome::ReplyProcessing, CallError> {
    Ok(email::outcome::progress(&ctx.db).await?)
}
pub async fn resume_reply_processing(ctx: &Context, _: Nothing) -> Result<(), CallError> {
    email::outcome::resume(&ctx.db).await?;
    ctx.reply_processor.restart();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[sqlx::test(migrator = "db::MIGRATOR")]
    async fn operations_expose_reply_only_fields_and_reject_non_reply_edits(db: SqlitePool) {
        sqlx::query(
            r#"INSERT INTO mailbox (id, address, sender_name, provider, smtp_host,
                smtp_port, smtp_security, imap_host, imap_port, imap_security, auth,
                secret, daily_limit, min_interval_s, rotation_order, created_at)
            VALUES (1, 'me@example.com', '学生', 'custom', 'localhost', 465, 'tls',
                'localhost', 993, 'tls', 'password', 'fixture', 50, 60, 1, 0)"#,
        )
        .execute(&db)
        .await
        .unwrap();
        let m = mentor::create(
            &db,
            &mentor::MentorFields {
                name: "导师".into(),
                ..Default::default()
            },
            1,
        )
        .await
        .unwrap();
        let mut ids = vec![];
        for (uid, kind) in ["reply", "auto_reply", "sent"].into_iter().enumerate() {
            let id = sqlx::query(
                r#"INSERT INTO message (mailbox_id, folder, uidvalidity, uid, kind,
                    sent_at, from_address, truncated)
                VALUES (1, 'INBOX', 1, ?, ?, 1, 'mentor@example.edu', 0)"#,
            )
            .bind(uid as i64)
            .bind(kind)
            .execute(&db)
            .await
            .unwrap()
            .last_insert_rowid();
            sqlx::query("INSERT INTO message_mentor(mentor_id,message_id) VALUES(?,?)")
                .bind(m)
                .bind(id)
                .execute(&db)
                .await
                .unwrap();
            ids.push(id);
        }
        let ctx = Context::new(db, net::Net::new(vec![]).unwrap(), std::path::Path::new(""));
        assert_eq!(
            call(&ctx, "getAiReplyOutcome", json!({})).await.unwrap(),
            false
        );
        call(&ctx, "setAiReplyOutcome", json!({"enabled":true}))
            .await
            .unwrap();
        assert_eq!(
            call(&ctx, "getAiReplyOutcome", json!({})).await.unwrap(),
            true
        );
        let entries = call(&ctx, "listCorrespondence", json!({"id":m}))
            .await
            .unwrap();
        assert_eq!(entries[0]["message"]["marking"], json!({"state":"marking"}));
        assert_eq!(entries[0]["message"]["outcome"], Value::Null);
        for i in [1, 2] {
            assert!(entries[i]["message"].get("outcome").is_none());
            assert!(entries[i]["message"].get("marking").is_none());
            let error = call(
                &ctx,
                "setReplyOutcome",
                json!({"messageId":ids[i],"outcome":"interested"}),
            )
            .await
            .unwrap_err();
            assert!(matches!(error.code, ErrorCode::Invalid));
            assert!(error.message.contains("导师的回信"));
        }
        let claim = email::outcome::next(&ctx.db).await.unwrap().unwrap();
        email::outcome::fail(&ctx.db, claim, &Failure::new("格式不对"))
            .await
            .unwrap();
        assert_eq!(
            call(&ctx, "replyProcessing", json!({})).await.unwrap()["failed"],
            1
        );
        call(&ctx, "retryReplyOutcome", json!({"messageId":ids[0]}))
            .await
            .unwrap();
        assert_eq!(
            call(&ctx, "replyProcessing", json!({})).await.unwrap()["remaining"],
            1
        );
        call(
            &ctx,
            "setReplyOutcome",
            json!({"messageId":ids[0],"outcome":"no_slot"}),
        )
        .await
        .unwrap();
        assert_eq!(
            call(&ctx, "replyProcessing", json!({})).await.unwrap()["remaining"],
            0
        );
        let entries = call(&ctx, "listCorrespondence", json!({"id":m}))
            .await
            .unwrap();
        assert_eq!(entries[0]["message"]["outcome"], "no_slot");
        assert_eq!(entries[0]["message"]["marking"], Value::Null);
        call(&ctx, "resumeReplyProcessing", json!({}))
            .await
            .unwrap();
        call(&ctx, "setAiReplyOutcome", json!({"enabled":false}))
            .await
            .unwrap();
    }
}
