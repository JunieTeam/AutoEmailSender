//! Tags on mentors.

use crate::*;

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TagMentors {
    pub tag_id: i64,
    pub mentor_ids: Vec<i64>,
    /// Adds the tag when true, removes it when false.
    pub on: bool,
}

#[derive(Debug, Deserialize, TS)]
pub struct TagId {
    pub id: i64,
}

/// A new tag, put straight on these mentors.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NewTag {
    #[serde(flatten)]
    pub form: mentor::tags::TagForm,
    pub mentor_ids: Vec<i64>,
}

#[derive(Debug, Deserialize, TS)]
pub struct TagName {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Deserialize, TS)]
pub struct TagColor {
    pub id: i64,
    pub color: String,
}

#[derive(Debug, Deserialize, TS)]
pub struct TagMove {
    pub id: i64,
    pub direction: mentor::tags::Direction,
}

pub const NO_TAG: &str = "找不到这个标签，可能已被删除";

pub async fn tag_mentors(ctx: &Context, input: TagMentors) -> Result<(), CallError> {
    Ok(mentor::tags::tag_mentors(&ctx.db, input.tag_id, &input.mentor_ids, input.on).await?)
}

pub async fn list_tags(ctx: &Context, _: Nothing) -> Result<Vec<mentor::tags::Tag>, CallError> {
    Ok(mentor::tags::list(&ctx.db).await?)
}

pub async fn create_tag(ctx: &Context, input: NewTag) -> Result<mentor::tags::Tag, CallError> {
    let fields = input.form.check().map_err(CallError::invalid)?;
    mentor::tags::create(&ctx.db, &fields, &input.mentor_ids)
        .await
        .map_err(|e| tag_error(e, &fields.name))
}

pub async fn rename_tag(ctx: &Context, input: TagName) -> Result<(), CallError> {
    let name = mentor::tags::checked_name(&input.name).map_err(CallError::invalid)?;
    let applied = mentor::tags::rename(&ctx.db, input.id, &name)
        .await
        .map_err(|e| tag_error(e, &name))?;
    require_applied(applied, NO_TAG)
}

pub async fn recolor_tag(ctx: &Context, input: TagColor) -> Result<(), CallError> {
    let color = mentor::tags::checked_color(&input.color).map_err(CallError::invalid)?;
    let applied = mentor::tags::recolor(&ctx.db, input.id, &color).await?;
    require_applied(applied, NO_TAG)
}

pub async fn move_tag(ctx: &Context, input: TagMove) -> Result<(), CallError> {
    Ok(mentor::tags::move_tag(&ctx.db, input.id, input.direction).await?)
}

pub async fn delete_tag(ctx: &Context, input: TagId) -> Result<(), CallError> {
    Ok(mentor::tags::delete(&ctx.db, input.id).await?)
}

pub fn tag_error(e: mentor::tags::TagError, name: &str) -> CallError {
    match e {
        mentor::tags::TagError::NameTaken => CallError::invalid(format!("已有名为“{name}”的标签")),
        mentor::tags::TagError::Db(e) => e.into(),
    }
}
