//! Letter templates.

use crate::*;

#[derive(Debug, Deserialize, Serialize, TS)]
pub struct TemplateId {
    pub id: i64,
}

#[derive(Debug, Deserialize, TS)]
pub struct TemplateEdit {
    pub id: i64,
    #[serde(flatten)]
    pub form: template::TemplateForm,
}

pub const NO_TEMPLATE: &str = "找不到这个模板，可能已被删除";

pub async fn list_placeholders(
    _: &Context,
    _: Nothing,
) -> Result<&'static [&'static str], CallError> {
    Ok(&template::PLACEHOLDERS)
}

pub async fn list_templates(
    ctx: &Context,
    _: Nothing,
) -> Result<Vec<template::TemplateSummary>, CallError> {
    Ok(template::list(&ctx.db).await?)
}

pub async fn get_template(
    ctx: &Context,
    input: TemplateId,
) -> Result<template::Template, CallError> {
    template::get(&ctx.db, input.id)
        .await?
        .ok_or_else(|| CallError::not_found(NO_TEMPLATE))
}

pub async fn add_template(
    ctx: &Context,
    form: template::TemplateForm,
) -> Result<TemplateId, CallError> {
    let fields = form.check().map_err(CallError::invalid)?;
    let id = template::create(&ctx.db, &fields, now()).await?;
    Ok(TemplateId { id })
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WordFile {
    pub name: String,
    pub html: String,
}

pub async fn import_word_template(_: &Context, input: UploadedFile) -> Result<WordFile, CallError> {
    let data = uploaded_bytes(&input.data)?;
    let Some((name, extension)) = input.file_name.trim().rsplit_once('.') else {
        return Err(CallError::invalid("请选择 DOCX 或 DOC 文件".into()));
    };
    if !extension.eq_ignore_ascii_case("docx") && !extension.eq_ignore_ascii_case("doc") {
        return Err(CallError::invalid("请选择 DOCX 或 DOC 文件".into()));
    }
    let unreadable = |detail: String| {
        CallError::invalid(
            "未能读取 Word 文件：文件可能已损坏、设有密码或不是 Word 格式，请换一个文件".into(),
        )
        .logging(format!("reading a Word template: {detail}"))
    };
    // Parsing is slow work, and a library may panic on a damaged file.
    let html = tokio::task::spawn_blocking(move || {
        material::word_body(&data)
            .map(|document| document.map(|body| body.to_html()))
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| unreadable(error.to_string()))?
    .map_err(unreadable)?
    .ok_or_else(|| CallError::invalid("文件中无可导入的文字，请换一个 Word 文件".into()))?;
    Ok(WordFile {
        name: name.into(),
        html,
    })
}

pub async fn update_template(ctx: &Context, input: TemplateEdit) -> Result<(), CallError> {
    let fields = input.form.check().map_err(CallError::invalid)?;
    let applied = template::update(&ctx.db, input.id, &fields, now()).await?;
    require_applied(applied, NO_TEMPLATE)
}

pub async fn delete_template(ctx: &Context, input: TemplateId) -> Result<(), CallError> {
    Ok(template::delete(&ctx.db, input.id).await?)
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TemplateForMentor {
    pub template_id: i64,
    pub mentor_id: i64,
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FilledTemplate {
    pub subject: String,
    pub body_md: String,
}

/// A template to make the default, or none.
#[derive(Debug, Deserialize, TS)]
pub struct DefaultTemplate {
    pub id: Option<i64>,
}

pub async fn set_default_template(ctx: &Context, input: DefaultTemplate) -> Result<(), CallError> {
    let applied = profile::set_default_template(&ctx.db, input.id).await?;
    require_applied(applied, NO_TEMPLATE)
}

pub async fn apply_template(
    ctx: &Context,
    input: TemplateForMentor,
) -> Result<FilledTemplate, CallError> {
    let id = input.template_id;
    let t = get_template(ctx, TemplateId { id }).await?;
    fill(ctx, &t, input.mentor_id).await
}

/// The default template filled in for the mentor, which a first email starts from; none when no
/// template is the default.
pub async fn apply_default_template(
    ctx: &Context,
    input: MentorId,
) -> Result<Option<FilledTemplate>, CallError> {
    match template::default(&ctx.db).await? {
        Some(t) => Ok(Some(fill(ctx, &t, input.id).await?)),
        None => Ok(None),
    }
}

/// The template with its placeholders filled in for the mentor.
pub async fn fill(
    ctx: &Context,
    t: &template::Template,
    mentor_id: i64,
) -> Result<FilledTemplate, CallError> {
    let m = get_mentor(ctx, MentorId { id: mentor_id }).await?;
    let me = profile::get(&ctx.db).await?;
    let filled = task::fill(&t.subject, &t.body_md, &m, me.name.as_deref());
    Ok(FilledTemplate {
        subject: filled.subject,
        body_md: filled.body_md,
    })
}
