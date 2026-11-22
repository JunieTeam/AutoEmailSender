//! Replies to a mentor's mail.

use crate::*;

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReplyTargetQuery {
    pub mentor_id: i64,
    pub id: Option<i64>,
}

pub async fn get_reply_target(
    ctx: &Context,
    input: ReplyTargetQuery,
) -> Result<Option<email::ReplyTarget>, CallError> {
    Ok(email::reply_target(&ctx.db, input.mentor_id, input.id).await?)
}

pub async fn get_reply_draft(
    ctx: &Context,
    input: MentorId,
) -> Result<Option<email::ReplyDraft>, CallError> {
    Ok(email::reply_draft(&ctx.db, input.id).await?)
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReplyDraftEdit {
    pub mentor_id: i64,
    pub reply_to: String,
    pub body_md: String,
    pub attachments: Vec<i64>,
}

pub async fn save_reply_draft(
    ctx: &Context,
    input: ReplyDraftEdit,
) -> Result<Option<email::ReplyDraft>, CallError> {
    let saved = email::save_reply_draft(
        &ctx.db,
        input.mentor_id,
        &input.reply_to,
        &input.body_md,
        &input.attachments,
        now(),
    )
    .await?;
    if saved.is_none() && (!input.body_md.trim().is_empty() || !input.attachments.is_empty()) {
        return Err(CallError::not_found(
            "回的那封信已不在这位导师的往来里，或导师已移入回收站。请选择另一封回信后再试。",
        ));
    }
    Ok(saved)
}

pub async fn check_reply_draft(
    ctx: &Context,
    input: ReplyDraftEdit,
) -> Result<Vec<check::Problem>, CallError> {
    let m = get_mentor(
        ctx,
        MentorId {
            id: input.mentor_id,
        },
    )
    .await?;
    // The subject belongs to the received message, not to the user's writing. Its placeholders
    // cannot be fixed here; checks apply only to the new body, never to the appended quote.
    let mut problems = check::letter(
        &m.name,
        &m.aliases,
        "",
        &input.body_md,
        &email::select_attachments(&mut *ctx.db.acquire().await?, &input.attachments)
            .await?
            .into_iter()
            .map(|a| a.size)
            .collect::<Vec<_>>(),
    );
    if let Some(draft) = email::reply_draft(&ctx.db, input.mentor_id).await? {
        let subject = draft
            .target
            .as_ref()
            .and_then(|t| t.subject.as_deref())
            .map(email::reply_subject)
            .unwrap_or_default();
        problems.extend(check::fact_check(
            draft.fact_check.as_ref(),
            &subject,
            &input.body_md,
        ));
    }
    Ok(problems)
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReplyEmail {
    pub mentor_id: i64,
    pub draft_id: i64,
    pub reply_to: String,
    pub body_md: String,
    /// Agent approvals receive these from the confirmation dialog.
    #[ts(optional)]
    pub html: Option<String>,
    #[ts(optional)]
    pub text: Option<String>,
    pub attachments: Vec<i64>,
}

pub async fn approve_reply(ctx: &Context, input: ReplyEmail) -> Result<(), CallError> {
    if input.body_md.trim().is_empty() {
        return Err(CallError::invalid("请先填写回复正文".into()));
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
        email::approve_reply(
            &ctx.db,
            input.mentor_id,
            input.draft_id,
            &input.reply_to,
            &approval,
            now(),
        )
        .await?,
        "这封回复已被批准、已被清空，或导师已移入回收站。请刷新往来后查看。",
        "有附件的材料已被删除。重新选择附件后再发送。",
    )
}

impl From<writing::reply::Error> for CallError {
    fn from(error: writing::reply::Error) -> Self {
        match error {
            writing::reply::Error::NoTarget => CallError::not_found(
                "回的那封信已删除或缺少邮件编号，无法在原线程里回复。请点另一封回信旁的“回复”；\
                缺少编号的请到邮箱里回复。",
            ),
            writing::reply::Error::Db(error) => error.into(),
        }
    }
}

pub async fn write_reply(
    ctx: &Context,
    Writing {
        request,
        edit: input,
    }: Writing<ReplyDraftEdit>,
) -> Result<writing::Letter, CallError> {
    let (turn, chosen) = ctx
        .writing_turn(request, || model::chosen(&ctx.db, model::Feature::Writing))
        .await?;
    let references = material::for_writing(&ctx.db).await?;
    let previous = writing::reply::history(&ctx.db, input.mentor_id, &input.reply_to).await?;
    let prepared = writing::prepare(
        &ctx.db,
        input.mentor_id,
        &input.attachments,
        writing::Basis {
            chosen: &chosen,
            references: &references,
            kind: writing::Kind::Reply(&previous),
        },
    )
    .await?;
    let email_id = email::reply_draft(&ctx.db, input.mentor_id)
        .await?
        .filter(|draft| draft.reply_to == input.reply_to)
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
