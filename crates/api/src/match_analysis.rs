//! Matching mentors against the student's background.

use crate::*;

#[derive(Debug, Deserialize, TS)]
pub struct MatchBatch {
    pub ids: Vec<i64>,
    pub redo: bool,
}

pub async fn plan_matching(
    ctx: &Context,
    input: MatchBatch,
) -> Result<matching::MatchEstimate, CallError> {
    Ok(matching::plan(&ctx.db, &input.ids, input.redo, now()).await?)
}
pub async fn start_matching(ctx: &Context, input: MatchBatch) -> Result<u64, CallError> {
    matching::ready(&ctx.db).await?;
    let count = mentor::matching::enqueue(&ctx.db, &input.ids, false, input.redo, now()).await?;
    ctx.matcher.restart();
    Ok(count)
}
pub async fn match_mentor(ctx: &Context, input: MentorId) -> Result<u64, CallError> {
    matching::ready(&ctx.db).await?;
    let m = get_mentor(ctx, MentorId { id: input.id }).await?;
    if m.trashed {
        return Err(CallError::invalid("请先从回收站恢复这位导师".into()));
    }
    if m.research.is_none() && m.papers.is_empty() {
        return Err(CallError::invalid(
            "请先补充导师的研究方向或代表论文".into(),
        ));
    }
    let count = mentor::matching::enqueue(&ctx.db, &[input.id], true, true, now()).await?;
    ctx.matcher.restart();
    Ok(count)
}

pub async fn matching_progress(
    ctx: &Context,
    _: Nothing,
) -> Result<mentor::matching::MatchProgress, CallError> {
    Ok(mentor::matching::progress(&ctx.db).await?)
}
pub async fn pause_matching(ctx: &Context, _: Nothing) -> Result<(), CallError> {
    Ok(mentor::matching::pause(&ctx.db, now(), None).await?)
}
pub async fn resume_matching(ctx: &Context, _: Nothing) -> Result<(), CallError> {
    matching::ready(&ctx.db).await?;
    mentor::matching::resume(&ctx.db).await?;
    ctx.matcher.restart();
    Ok(())
}
pub async fn cancel_matching(ctx: &Context, _: Nothing) -> Result<(), CallError> {
    mentor::matching::cancel(&ctx.db).await?;
    ctx.matcher.wake();
    Ok(())
}
pub async fn retry_matching(ctx: &Context, _: Nothing) -> Result<u64, CallError> {
    matching::ready(&ctx.db).await?;
    let count = mentor::matching::retry(&ctx.db, now()).await?;
    ctx.matcher.restart();
    Ok(count)
}
impl From<matching::Error> for CallError {
    fn from(e: matching::Error) -> Self {
        match e {
            matching::Error::NoModel => Self::invalid(matching::NO_MODEL.into()),
            matching::Error::NoBasis => Self::invalid(matching::NO_BASIS.into()),
            matching::Error::NotAgreed => {
                Self::new(ErrorCode::NeedsAgreement, matching::NOT_AGREED)
            }
            matching::Error::Db(e) => e.into(),
        }
    }
}
