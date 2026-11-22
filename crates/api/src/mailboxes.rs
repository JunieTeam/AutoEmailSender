//! The student's mailboxes and test mail.

use crate::*;

#[derive(Debug, Deserialize, Serialize, TS)]
pub struct MailboxId {
    pub id: i64,
}

#[derive(Debug, Deserialize, TS)]
pub struct NewMailbox {
    #[serde(flatten)]
    pub form: mailbox::MailboxForm,
    pub secret: String,
}

#[derive(Debug, Deserialize, TS)]
pub struct MailboxEdit {
    pub id: i64,
    #[serde(flatten)]
    pub form: mailbox::MailboxForm,
    /// Blank keeps the saved one, unless the servers changed.
    pub secret: String,
}

/// A mailbox as filled in, perhaps not saved yet.
#[derive(Debug, Deserialize, TS)]
pub struct MailboxTest {
    /// The saved mailbox being edited, whose secret a blank one stands for while the servers stay
    /// the same.
    #[ts(optional)]
    pub id: Option<i64>,
    #[serde(flatten)]
    pub form: mailbox::MailboxForm,
    pub secret: String,
}

pub const NO_MAILBOX: &str = "找不到这个邮箱，可能已被删除";
pub const NO_SECRET: &str = "请填写授权码或密码";

pub async fn list_providers(
    _: &Context,
    _: Nothing,
) -> Result<&'static [mailbox::providers::Provider], CallError> {
    Ok(mailbox::providers::PROVIDERS)
}

pub async fn list_mailboxes(ctx: &Context, _: Nothing) -> Result<Vec<mailbox::Mailbox>, CallError> {
    Ok(mailbox::list(&ctx.db, now()).await?)
}

pub async fn get_rotation(ctx: &Context, _: Nothing) -> Result<profile::Rotation, CallError> {
    Ok(profile::rotation(&ctx.db).await?)
}

pub async fn set_rotation(ctx: &Context, rotation: profile::Rotation) -> Result<(), CallError> {
    profile::set_rotation(&ctx.db, rotation).await?;
    ctx.sender.wake();
    Ok(())
}

pub async fn add_mailbox(ctx: &Context, input: NewMailbox) -> Result<MailboxId, CallError> {
    let fields = input.form.check().map_err(CallError::invalid)?;
    let secret = fields
        .secret(&input.secret)
        .map_err(|e| CallError::invalid(e.into()))?
        .ok_or_else(|| CallError::invalid(NO_SECRET.into()))?;
    let id = mailbox::create(&ctx.db, &fields, &secret, now()).await?;
    // Emails may be waiting for a mailbox.
    ctx.sender.wake();
    ctx.receiver.wake();
    Ok(MailboxId { id })
}

pub async fn update_mailbox(ctx: &Context, input: MailboxEdit) -> Result<(), CallError> {
    let fields = input.form.check().map_err(CallError::invalid)?;
    let secret = entered_or_saved_secret(ctx, &input.secret, Some(input.id), &fields).await?;
    let held = ctx.receiver.hold().await;
    let updated = mailbox::update(&ctx.db, input.id, &fields, &secret).await?;
    drop(held);
    require_applied(updated, NO_MAILBOX)?;
    // A higher limit or a shorter interval may let a waiting email go, and receiving that
    // failed may work now. What changed goes up to the official server for its emails.
    ctx.sender.wake();
    ctx.receiver.wake();
    ctx.cloud_send.wake();
    Ok(())
}

pub async fn resume_mailbox(ctx: &Context, input: MailboxId) -> Result<(), CallError> {
    require_applied(mailbox::resume(&ctx.db, input.id).await?, NO_MAILBOX)?;
    ctx.sender.wake();
    Ok(())
}

pub async fn delete_mailbox(ctx: &Context, input: MailboxId) -> Result<(), CallError> {
    let _held = ctx.receiver.hold().await;
    mailbox::delete(&ctx.db, input.id).await?;
    // Its mentors' first emails can now go from the other mailboxes, and replies from it fail.
    ctx.sender.wake();
    Ok(())
}

pub async fn test_sending(ctx: &Context, input: MailboxTest) -> Result<(), CallError> {
    let id = input.id;
    let account = test_account(ctx, input).await?;
    let smtp = &account.sender.smtp;
    transport::check_sending(&ctx.net, &account.sender)
        .await
        .map_err(|e| {
            CallError::from(e).logging(format!("mailbox {id:?} at {}:{}", smtp.host, smtp.port))
        })
}

pub async fn test_receiving(ctx: &Context, input: MailboxTest) -> Result<(), CallError> {
    let id = input.id;
    let account = test_account(ctx, input).await?;
    let imap = &account.imap;
    mailbox::check_receiving(&ctx.net, &account)
        .await
        .map_err(|e| {
            CallError::from(e).logging(format!("mailbox {id:?} at {}:{}", imap.host, imap.port))
        })
}

pub async fn test_account(
    ctx: &Context,
    input: MailboxTest,
) -> Result<mailbox::Account, CallError> {
    let fields = input.form.check().map_err(CallError::invalid)?;
    let secret = entered_or_saved_secret(ctx, &input.secret, input.id, &fields).await?;
    Ok(fields.account(secret))
}

/// The secret typed in, else the saved one of the mailbox being edited if its servers stay the
/// same.
pub async fn entered_or_saved_secret(
    ctx: &Context,
    secret: &str,
    id: Option<i64>,
    fields: &mailbox::MailboxFields,
) -> Result<String, CallError> {
    if let Some(secret) = fields
        .secret(secret)
        .map_err(|e| CallError::invalid(e.into()))?
    {
        return Ok(secret);
    }
    let saved = match id {
        Some(id) => mailbox::secret(&ctx.db, id, fields).await?,
        None => None,
    };
    saved.ok_or_else(|| CallError::invalid(NO_SECRET.into()))
}

pub async fn send_test_mail(ctx: &Context, input: MailboxId) -> Result<(), CallError> {
    let account = mailbox::account(&ctx.db, input.id).await?.ok_or_else(|| {
        CallError::invalid("请先在“设置 › 邮件 › 邮箱”填写授权码或密码，再发测试信".into())
    })?;
    Ok(transport::send_test(&ctx.net, &account.sender).await?)
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SendTestLetter {
    pub mailbox_id: i64,
    pub subject: String,
    pub html: String,
    pub text: String,
    pub attachments: Vec<i64>,
}

pub async fn send_test_letter(ctx: &Context, input: SendTestLetter) -> Result<(), CallError> {
    let account = mailbox::account(&ctx.db, input.mailbox_id)
        .await?
        .ok_or_else(|| CallError::not_found(NO_MAILBOX))?;
    let ids = sqlx::types::Json(&input.attachments);
    let rows = sqlx::query!(
        r#"SELECT m.id AS "id?", m.file_name FROM json_each(?) j
           LEFT JOIN material m ON m.id = j.value ORDER BY j.key"#,
        ids,
    )
    .fetch_all(&ctx.db)
    .await?;
    let mut attached: Vec<email::Attached> = Vec::new();
    for row in rows {
        let (Some(material_id), Some(file_name)) = (row.id, row.file_name) else {
            return Err(CallError::invalid(
                "附件材料已被删除。请重新选择附件后再发送。".into(),
            ));
        };
        // As with an approved letter, the first selected file with each name is kept.
        if !attached.iter().any(|a| a.file_name == file_name) {
            attached.push(email::Attached {
                material_id,
                file_name,
            });
        }
    }
    let files = send::files(&ctx.materials, &attached)
        .await
        .map_err(CallError::invalid)?;
    let sender = &account.sender;
    let message_id = transport::message_id(sender);
    let outgoing = transport::compose(
        sender,
        &transport::Letter {
            message_id: &message_id,
            in_reply_to: None,
            references: None,
            to_name: &sender.sender_name,
            to: sender.address.as_ref(),
            subject: &input.subject,
            html: &input.html,
            text: &input.text,
            attachments: &files,
        },
    )?;
    transport::login(&ctx.net, sender)
        .await?
        .send(&outgoing, || false)
        .await
        .map_err(|e| {
            CallError::external(e.failure(), diagnosed(&e, e.diagnostic()))
                .logging(format!("test letter from mailbox {}", input.mailbox_id))
        })?;
    tracing::info!("test letter sent from mailbox {}", input.mailbox_id);
    Ok(())
}

impl From<mailbox::SaveError> for CallError {
    fn from(e: mailbox::SaveError) -> Self {
        match e {
            mailbox::SaveError::AddressTaken => CallError::invalid("这个邮箱已添加".into()),
            mailbox::SaveError::InCloud(count) => CallError::invalid(format!(
                "这个邮箱还有 {count} 封邮件已交给云端，要改地址或删除邮箱，请先把它们撤回到本地"
            )),
            mailbox::SaveError::Db(e) => e.into(),
        }
    }
}

impl From<transport::MailError> for CallError {
    fn from(e: transport::MailError) -> Self {
        CallError::external(e.failure(), diagnosed(&e, e.diagnostic()))
    }
}
