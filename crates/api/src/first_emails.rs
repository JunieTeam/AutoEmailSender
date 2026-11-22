//! Writing, checking and approving a mentor's first email.

use crate::*;

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DraftMaterial {
    pub mentor_id: i64,
    pub kind: email::DraftKind,
    pub material_id: i64,
}

pub async fn attach_draft_material(ctx: &Context, input: DraftMaterial) -> Result<bool, CallError> {
    Ok(
        email::attach_draft_material(&ctx.db, input.mentor_id, input.kind, input.material_id)
            .await?,
    )
}

pub async fn get_first_draft(
    ctx: &Context,
    input: MentorId,
) -> Result<Option<email::FirstDraft>, CallError> {
    Ok(email::first_draft(&ctx.db, input.id).await?)
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FirstDraftEdit {
    pub mentor_id: i64,
    pub subject: String,
    pub body_md: String,
    /// Materials, by id.
    pub attachments: Vec<i64>,
}

pub async fn save_first_draft(
    ctx: &Context,
    input: FirstDraftEdit,
) -> Result<Option<email::FirstDraft>, CallError> {
    let saved = email::save_first_draft(
        &ctx.db,
        input.mentor_id,
        &input.subject,
        &input.body_md,
        &input.attachments,
        now(),
    )
    .await?;
    if saved.is_none() && (!input.subject.trim().is_empty() || !input.body_md.trim().is_empty()) {
        return Err(CallError::invalid(
            "还有一封未发出的首封或追问，请先在任务或往来里处理".into(),
        ));
    }
    Ok(saved)
}

/// The draft as it is in the composer, which may be ahead of the one saved.
pub async fn check_first_draft(
    ctx: &Context,
    input: FirstDraftEdit,
) -> Result<Vec<check::Problem>, CallError> {
    let id = input.mentor_id;
    let m = get_mentor(ctx, MentorId { id }).await?;
    let mut problems = check::letter(
        &m.name,
        &m.aliases,
        &input.subject,
        &input.body_md,
        &email::select_attachments(&mut *ctx.db.acquire().await?, &input.attachments)
            .await?
            .into_iter()
            .map(|a| a.size)
            .collect::<Vec<_>>(),
    );
    let (count, last) = match m.contact {
        mentor::Contact::None => (0, None),
        mentor::Contact::Contacted {
            count, last_out_at, ..
        }
        | mentor::Contact::Bounced { count, last_out_at } => (count, Some(last_out_at)),
        mentor::Contact::Replied {
            count, last_out_at, ..
        } => (count, last_out_at),
    };
    problems.extend(check::recent(count, last, now()));
    if let Some(draft) = email::first_draft(&ctx.db, id).await? {
        problems.extend(check::fact_check(
            draft.fact_check.as_ref(),
            &input.subject,
            &input.body_md,
        ));
    }
    Ok(problems)
}

/// Drafts the letter when it is blank and rewrites it otherwise.
pub async fn write_letter(
    ctx: &Context,
    Writing {
        request,
        edit: input,
    }: Writing<FirstDraftEdit>,
) -> Result<writing::Letter, CallError> {
    let (turn, chosen) = ctx
        .writing_turn(request, || model::chosen(&ctx.db, model::Feature::Writing))
        .await?;
    let current = writing::Letter {
        subject: input.subject,
        body_md: input.body_md,
    };
    let references = material::for_writing(&ctx.db).await?;
    let prepared = writing::prepare(
        &ctx.db,
        input.mentor_id,
        &input.attachments,
        writing::Basis {
            chosen: &chosen,
            references: &references,
            kind: writing::Kind::First,
        },
    )
    .await?;
    let email_id = email::first_draft(&ctx.db, input.mentor_id)
        .await?
        .map(|draft| draft.id);
    let about = About {
        crawl: None,
        mentor: Some(input.mentor_id),
        email: email_id,
    };
    let model = turn.send()?;
    writing::write(&ctx.net, model, &prepared, &current, |purpose, call| {
        model::usage::record(&ctx.db, &chosen, purpose, about, call)
    })
    .await
    .map_err(|e| writing_failed(e, input.mentor_id, chosen.id))
}

pub fn writing_failed(e: writing::Error, mentor: i64, model_config: i64) -> CallError {
    CallError::from(e).logging(format!("mentor {mentor}, model config {model_config}"))
}

impl From<writing::Error> for CallError {
    fn from(e: writing::Error) -> Self {
        match e {
            writing::Error::NoModel => {
                CallError::invalid("尚未添加模型。在“设置 › AI › 模型”里添加后再试。".into())
            }
            writing::Error::NotAgreed => {
                CallError::new(ErrorCode::NeedsAgreement, matching::NOT_AGREED)
            }
            writing::Error::NoMentor => CallError::not_found(NO_MENTOR),
            writing::Error::Model(e) => e.into(),
            writing::Error::NotALetter { finish_reason } => {
                let message = match finish_reason {
                    Some(model::Finish::Length) => model::CUT_OFF,
                    _ => "模型未按要求的格式回答，请重试或更换模型",
                };
                CallError::external(
                    Failure::new(message),
                    format!("the model's answer is not a letter ({finish_reason:?})"),
                )
            }
            writing::Error::Db(e) => e.into(),
        }
    }
}

#[derive(Debug, Deserialize, TS)]
pub struct MailAddress {
    pub address: String,
}

/// A lookup that fails, such as when offline, tells nothing: the interface shows no error and
/// asks again later (docs/design.md#联系).
pub async fn check_mail_domain(
    ctx: &Context,
    input: MailAddress,
) -> Result<Option<check::Problem>, CallError> {
    ctx.mail_domains.check(&input.address).await
}

/// The mentor's first draft as the user approved it, with the HTML and plain text the interface
/// rendered from it in the letter style.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FirstEmail {
    pub planned_at: Option<i64>,
    pub mentor_id: i64,
    pub subject: String,
    pub body_md: String,
    /// Agent approvals receive these from the confirmation dialog.
    #[ts(optional)]
    pub html: Option<String>,
    #[ts(optional)]
    pub text: Option<String>,
    /// Materials, by id.
    pub attachments: Vec<i64>,
}

pub async fn approve_first_email(ctx: &Context, input: FirstEmail) -> Result<(), CallError> {
    let subject = input.subject.trim();
    if subject.is_empty() || input.body_md.trim().is_empty() {
        return Err(CallError::invalid("主题和正文都要填写".into()));
    }
    let id = input.mentor_id;
    let m = get_mentor(ctx, MentorId { id }).await?;
    if m.emails.is_empty() {
        return Err(CallError::invalid(
            "导师尚未填写邮箱。在导师资料里填写后再发送。".into(),
        ));
    }
    if mailbox::list(&ctx.db, now()).await?.is_empty() {
        return Err(CallError::invalid(
            "尚未添加发信的邮箱。在“设置 › 邮件 › 邮箱”里添加后再发送。".into(),
        ));
    }
    let approval = email::Approval {
        subject,
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
        email::approve_first(&ctx.db, input.mentor_id, &approval, input.planned_at, now()).await?,
        "这位导师已经有一封首封邮件，或已移入回收站",
        "有附件的材料已被删除。重新选择附件后再发送。",
    )
}

pub fn approved(
    ctx: &Context,
    result: email::Approved,
    refused: &str,
    material_gone: &str,
) -> Result<(), CallError> {
    match result {
        email::Approved::Yes => {
            ctx.sender.wake();
            Ok(())
        }
        email::Approved::TimePassed => Err(CallError::invalid(
            "所选时间已过，请选择未来的日期和时间".into(),
        )),
        email::Approved::Refused => Err(CallError::not_found(refused)),
        email::Approved::FollowUpReplied => {
            Err(CallError::invalid("导师已回信，请到往来里回复".into()))
        }
        email::Approved::FollowUpParentBounced => Err(CallError::invalid(
            "上一封被退回，请先核对导师的邮箱".into(),
        )),
        email::Approved::MaterialGone => Err(CallError::invalid(material_gone.into())),
        email::Approved::ReplyGone => Err(CallError::not_found(
            "回的那封信已不在这位导师的往来里。请选择另一封回信后再发送。",
        )),
        email::Approved::ReplyChanged => {
            Err(CallError::invalid("回复的信已更换，请再看一遍".into()))
        }
        email::Approved::NoAddress => Err(CallError::invalid(
            "导师尚未填写邮箱。在导师资料里填写后再发送。".into(),
        )),
    }
}
