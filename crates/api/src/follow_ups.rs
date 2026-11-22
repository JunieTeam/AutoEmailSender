//! Follow-ups to a mentor who has not answered.

use crate::*;

impl From<email::follow_up::Error> for CallError {
    fn from(e: email::follow_up::Error) -> Self {
        match e {
            email::follow_up::Error::Invalid(s) => Self::invalid(s),
            email::follow_up::Error::Db(e) => e.into(),
        }
    }
}
pub async fn get_follow_up_entry(
    ctx: &Context,
    key: MentorId,
) -> Result<email::follow_up::FollowUpEntry, CallError> {
    Ok(email::follow_up::entry(&ctx.db, key.id).await?)
}
pub async fn get_follow_up_draft(
    ctx: &Context,
    key: MentorId,
) -> Result<Option<email::follow_up::FollowUpDraft>, CallError> {
    Ok(email::follow_up::draft(&ctx.db, key.id).await?)
}
pub async fn create_follow_up_draft(
    ctx: &Context,
    key: MentorId,
) -> Result<email::follow_up::FollowUpDraft, CallError> {
    Ok(email::follow_up::create(&ctx.db, key.id, now()).await?)
}
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FollowUpEdit {
    pub draft_id: i64,
    #[serde(flatten)]
    pub letter: ReplyDraftEdit,
}
pub async fn save_follow_up_draft(
    ctx: &Context,
    input: FollowUpEdit,
) -> Result<Option<email::follow_up::FollowUpDraft>, CallError> {
    let d = input.letter;
    let saved = email::follow_up::save(
        &ctx.db,
        d.mentor_id,
        input.draft_id,
        &d.reply_to,
        &d.body_md,
        &d.attachments,
        now(),
    )
    .await?;
    if saved.is_none() {
        return Err(CallError::invalid(
            "追问已取消或已发送，请刷新往来后查看".into(),
        ));
    }
    Ok(saved)
}
pub async fn approve_follow_up(ctx: &Context, input: ReplyEmail) -> Result<(), CallError> {
    if input.body_md.trim().is_empty() {
        return Err(CallError::invalid("请先填写追问正文".into()));
    }
    let approval = email::Approval {
        subject: "",
        body_md: &input.body_md,
        html: input
            .html
            .as_deref()
            .ok_or_else(|| CallError::invalid("缺少邮件的 HTML 渲染结果".into()))?,
        text: input
            .text
            .as_deref()
            .ok_or_else(|| CallError::invalid("缺少邮件的纯文本渲染结果".into()))?,
        attachments: &input.attachments,
    };
    approved(
        ctx,
        email::follow_up::approve(
            &ctx.db,
            input.mentor_id,
            input.draft_id,
            &input.reply_to,
            &approval,
            now(),
        )
        .await?,
        "追问已取消、已发送，或导师已无法追问，请刷新往来后查看",
        "有附件的材料已被删除。重新选择附件后再发送。",
    )
}
pub async fn write_follow_up(
    ctx: &Context,
    Writing {
        request,
        edit: input,
    }: Writing<FirstDraftEdit>,
) -> Result<writing::Letter, CallError> {
    let (turn, chosen) = ctx
        .writing_turn(request, || model::chosen(&ctx.db, model::Feature::Writing))
        .await?;
    let references = material::for_writing(&ctx.db).await?;
    let previous = task::history(&mut *ctx.db.acquire().await?, input.mentor_id).await?;
    let prepared = writing::prepare(
        &ctx.db,
        input.mentor_id,
        &input.attachments,
        writing::Basis {
            chosen: &chosen,
            references: &references,
            kind: writing::Kind::FollowUp(&previous),
        },
    )
    .await?;
    let email_id = email::follow_up::draft(&ctx.db, input.mentor_id)
        .await?
        .map(|draft| draft.id);
    let about = About {
        crawl: None,
        mentor: Some(input.mentor_id),
        email: email_id,
    };
    let model = turn.send()?;
    writing::write(
        &ctx.net,
        model,
        &prepared,
        &writing::Letter {
            subject: String::new(),
            body_md: input.body_md,
        },
        |purpose, call| model::usage::record(&ctx.db, &chosen, purpose, about, call),
    )
    .await
    .map_err(|e| writing_failed(e, input.mentor_id, chosen.id))
}

pub async fn discard_follow_up_draft(ctx: &Context, key: EmailId) -> Result<(), CallError> {
    require_applied(
        email::follow_up::discard(&ctx.db, key.id, now()).await?,
        "追问已取消或已发送，请刷新往来后查看",
    )
}
