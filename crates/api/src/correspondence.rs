//! The emails and mail exchanged with a mentor.

use crate::*;

/// An email the app sent, or mail seen in the mailbox, in the correspondence with a mentor.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum Entry {
    Email(email::Email),
    Message(receive::Message),
    Todo(email::todos::Todo),
}

/// The correspondence with a mentor, oldest first; emails still to go out come last.
pub async fn list_correspondence(ctx: &Context, input: MentorId) -> Result<Vec<Entry>, CallError> {
    let mut tx = ctx.db.begin().await?;
    let emails = email::list_in(&mut tx, input.id).await?;
    let messages = receive::list_in(&mut tx, input.id).await?;
    let mut entries: Vec<(i64, Entry)> = emails
        .into_iter()
        .map(|e| {
            let at = match e.delivery {
                email::Delivery::MaybeSent { sent_at, .. }
                | email::Delivery::Sent { sent_at, .. }
                | email::Delivery::Bounced { sent_at, .. } => sent_at,
                email::Delivery::Approved
                | email::Delivery::Cloud { .. }
                | email::Delivery::Sending { .. }
                | email::Delivery::Failed { .. }
                | email::Delivery::Canceled { .. } => i64::MAX,
            };
            (at, Entry::Email(e))
        })
        .chain(messages.into_iter().map(|m| (m.sent_at, Entry::Message(m))))
        .collect();
    let todos = email::todos::for_mentor(&mut *tx, input.id).await?;
    let mut from_replies: BTreeMap<i64, Vec<email::todos::Todo>> = BTreeMap::new();
    for todo in todos {
        if let Some(id) = todo.message_id {
            from_replies.entry(id).or_default().push(todo);
        } else {
            entries.push((todo.created_at, Entry::Todo(todo)));
        }
    }
    entries.sort_by_key(|(at, _)| *at);
    let mut result = Vec::new();
    for (_, entry) in entries {
        let message = match &entry {
            Entry::Message(m) => Some(m.id),
            _ => None,
        };
        result.push(entry);
        if let Some(todos) = message.and_then(|id| from_replies.remove(&id)) {
            result.extend(todos.into_iter().map(Entry::Todo));
        }
    }
    Ok(result)
}

#[derive(Debug, Deserialize, TS)]
pub struct MessageIds {
    pub ids: Vec<i64>,
}

pub async fn mark_read(ctx: &Context, input: MessageIds) -> Result<(), CallError> {
    Ok(receive::mark_read(&ctx.db, &input.ids, now()).await?)
}

#[derive(Debug, Deserialize, TS)]
pub struct EmailId {
    pub id: i64,
}

pub async fn withdraw_email(ctx: &Context, input: EmailId) -> Result<(), CallError> {
    let applied = email::withdraw(&ctx.db, input.id, now())
        .await
        .map_err(|e| {
            if e.as_database_error()
                .is_some_and(|e| e.is_unique_violation())
            {
                CallError::invalid(
                    "这位导师已有同类草稿。请先完成或清空那封草稿，再改回这封。".into(),
                )
            } else {
                e.into()
            }
        })?;
    require_applied(applied, "这封邮件已在发送或已发出，无法改回草稿")?;
    ctx.sender.wake();
    Ok(())
}

pub async fn retry_email(ctx: &Context, input: EmailId) -> Result<(), CallError> {
    let applied = email::retry(&ctx.db, input.id, now()).await?;
    require_applied(applied, "这封邮件已取消，或已不在可重试状态")?;
    ctx.sender.wake();
    Ok(())
}

pub async fn confirm_unsent_email(ctx: &Context, input: EmailId) -> Result<(), CallError> {
    require_applied(
        email::confirm_unsent(&ctx.db, input.id, now()).await?,
        "这封邮件无待确认的取消记录，或发送状态已变化",
    )
}

pub async fn resend_email(ctx: &Context, input: EmailId) -> Result<(), CallError> {
    approved(
        ctx,
        email::resend(&ctx.db, input.id, now()).await?,
        "这封邮件已经重新发送过，或导师已移入回收站",
        "附件对应的材料已被删除，无法按原样重新发送",
    )
}
