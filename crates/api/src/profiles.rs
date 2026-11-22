//! The student's profile and letter style.

use crate::*;

pub async fn get_profile(ctx: &Context, _: Nothing) -> Result<profile::Profile, CallError> {
    Ok(profile::get(&ctx.db).await?)
}

pub async fn update_profile(
    ctx: &Context,
    form: profile::ProfileForm,
) -> Result<profile::Profile, CallError> {
    Ok(profile::update(&ctx.db, &form, now()).await?)
}

pub async fn finish_onboarding(ctx: &Context, _: Nothing) -> Result<(), CallError> {
    Ok(profile::finish_onboarding(&ctx.db).await?)
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TestLetter {
    pub template_name: String,
    pub subject: String,
    pub body_md: String,
    pub attachments: Vec<i64>,
}

pub async fn test_letter(ctx: &Context, _: Nothing) -> Result<Option<TestLetter>, CallError> {
    let mut tx = ctx.db.begin().await?;
    let Some(t) = template::default_or_first(&mut tx).await? else {
        return Ok(None);
    };
    let name = sqlx::query_scalar!("SELECT name FROM profile")
        .fetch_one(&mut *tx)
        .await?;
    let attachments =
        sqlx::query_scalar!("SELECT id FROM material WHERE attach_by_default = 1 ORDER BY id")
            .fetch_all(&mut *tx)
            .await?;
    tx.commit().await?;
    let fill = template::Fill {
        my_name: name.as_deref(),
        ..template::EXAMPLE_MENTOR
    };
    Ok(Some(TestLetter {
        template_name: t.name,
        subject: fill.subject(&t.subject),
        body_md: fill.body(&t.body_md),
        attachments,
    }))
}

pub async fn get_letter_style(
    ctx: &Context,
    _: Nothing,
) -> Result<profile::LetterStyle, CallError> {
    Ok(profile::letter_style(&ctx.db).await?)
}

pub async fn update_letter_style(
    ctx: &Context,
    style: profile::LetterStyle,
) -> Result<(), CallError> {
    style.check().map_err(CallError::invalid)?;
    Ok(profile::update_letter_style(&ctx.db, &style).await?)
}
