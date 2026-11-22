//! Receiving mail and the notifications it brings.

use crate::*;

pub async fn list_conversations(
    ctx: &Context,
    query: mentor::conversations::ConversationQuery,
) -> Result<mentor::conversations::ConversationPage, CallError> {
    Ok(mentor::conversations::list(&ctx.db, &query).await?)
}

pub async fn unread_count(ctx: &Context, _: Nothing) -> Result<i64, CallError> {
    Ok(mentor::conversations::unread_count(&ctx.db).await?)
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReceiveStatus {
    pub received_at: Option<i64>,
}

pub async fn receive_status(ctx: &Context, _: Nothing) -> Result<ReceiveStatus, CallError> {
    Ok(ReceiveStatus {
        received_at: ctx.receiver.received_at(),
    })
}

pub async fn receive_now(ctx: &Context, input: Nothing) -> Result<ReceiveStatus, CallError> {
    ctx.receiver
        .receive_now()
        .await
        .map_err(|e| CallError::internal(format!("database: {e}")))?;
    receive_status(ctx, input).await
}

#[derive(Debug, Deserialize, TS)]
pub struct NotificationSetting {
    pub on: bool,
}

pub async fn get_reply_notifications(ctx: &Context, _: Nothing) -> Result<bool, CallError> {
    Ok(profile::reply_notifications(&ctx.db).await?)
}

pub async fn set_reply_notifications(
    ctx: &Context,
    input: NotificationSetting,
) -> Result<(), CallError> {
    Ok(profile::set_reply_notifications(&ctx.db, input.on).await?)
}

pub async fn get_background_notifications(ctx: &Context, _: Nothing) -> Result<bool, CallError> {
    Ok(profile::background_notifications(&ctx.db).await?)
}
pub async fn set_background_notifications(
    ctx: &Context,
    input: NotificationSetting,
) -> Result<(), CallError> {
    Ok(profile::set_background_notifications(&ctx.db, input.on).await?)
}
