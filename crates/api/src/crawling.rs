use super::*;
use crawl::{queue, review};
#[derive(Debug, Deserialize, Serialize, TS)]
pub struct CrawlId {
    pub id: i64,
}
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CrawlStart {
    pub urls: String,
    pub model_id: i64,
}
#[derive(Debug, Deserialize, TS)]
pub struct CrawlQuery {
    pub page: u32,
}
#[derive(Debug, Deserialize, TS)]
pub struct CrawlDetailQuery {
    pub id: i64,
    pub page: u32,
    /// Only the pages that did not open.
    #[serde(default)]
    pub failed: bool,
}
#[derive(Debug, Deserialize, TS)]
pub struct CrawlRetry {
    pub id: i64,
    pub page: Option<i64>,
}
#[derive(Debug, Deserialize, TS)]
pub struct CandidateIds {
    pub ids: Vec<i64>,
}
#[derive(Debug, Deserialize, TS)]
pub struct CrawlUnit {
    pub id: i64,
    pub university: String,
    pub school: Option<String>,
}
pub(super) async fn set_crawl_unit(ctx: &Context, input: CrawlUnit) -> Result<(), CallError> {
    let text = |s: &str| Some(s.trim()).filter(|s| !s.is_empty()).map(str::to_owned);
    let university =
        text(&input.university).ok_or_else(|| CallError::invalid("请填写学校".into()))?;
    let unit = mentor::Affiliation {
        university: Some(university),
        school: input.school.as_deref().and_then(text),
        ..Default::default()
    };
    match review::set_unit(&ctx.db, input.id, &unit).await? {
        review::UnitChange::Set => Ok(()),
        review::UnitChange::Deleted => Err(CallError::not_found("这次整理已被删除")),
        review::UnitChange::NotSite => Err(CallError::invalid(
            "只有智能整理的记录能改学校和学院；Agent 提交和补全已有导师的候选按各自的单位".into(),
        )),
    }
}
pub(super) async fn submit_candidates(
    ctx: &Context,
    input: crawl::submit::SubmitCandidates,
) -> Result<crawl::submit::Submitted, CallError> {
    crawl::submit::submit(&ctx.db, input, now())
        .await
        .map_err(|e| match e {
            crawl::submit::SubmitError::Invalid(message) => CallError::invalid(message),
            crawl::submit::SubmitError::Deleted => CallError::not_found("这次整理已被删除"),
            crawl::submit::SubmitError::Db(error) => error.into(),
        })
}
async fn crawl_ready(ctx: &Context, id: Option<i64>) -> Result<model::Chosen, CallError> {
    let m = model::selected(&ctx.db, id, model::Feature::Crawl)
        .await?
        .ok_or_else(|| {
            CallError::invalid("尚未添加模型，请在“设置 › AI › 模型”中添加后再开始".into())
        })?;
    if !m.agreed {
        return Err(CallError::new(
            ErrorCode::NeedsAgreement,
            "网页的标题、文字、链接文字及公开的导师资料将发给这个模型的服务商，\
                用于寻找名单和整理资料。",
        ));
    }
    Ok(m)
}
#[derive(Debug, Deserialize, TS)]
pub struct CrawlUrls {
    pub urls: String,
}
#[derive(Debug, Deserialize, TS)]
pub struct CrawlWorkQuery {
    pub start: i64,
    pub end: i64,
}
pub(super) async fn check_crawl_urls(
    _: &Context,
    input: CrawlUrls,
) -> Result<Vec<String>, CallError> {
    Ok(queue::starts(&input.urls)
        .map_err(CallError::invalid)?
        .iter()
        .map(|u| u.as_str().to_owned())
        .collect())
}
pub(super) async fn crawl_work(
    ctx: &Context,
    input: CrawlWorkQuery,
) -> Result<Vec<queue::CrawlSummary>, CallError> {
    Ok(queue::work(&ctx.db, input.start, input.end).await?)
}
pub(super) async fn start_crawl(ctx: &Context, input: CrawlStart) -> Result<CrawlId, CallError> {
    let urls = queue::starts(&input.urls).map_err(CallError::invalid)?;
    let m = crawl_ready(ctx, Some(input.model_id)).await?;
    let id = queue::start(&ctx.db, &urls, m.id, now()).await?;
    ctx.crawler.wake();
    Ok(CrawlId { id })
}
pub(super) async fn list_crawls(
    ctx: &Context,
    input: CrawlQuery,
) -> Result<queue::CrawlList, CallError> {
    Ok(queue::list(&ctx.db, input.page).await?)
}
pub(super) async fn crawl_options(
    ctx: &Context,
    _: Nothing,
) -> Result<Vec<queue::CrawlOption>, CallError> {
    Ok(queue::options(&ctx.db).await?)
}
pub(super) async fn get_crawl(
    ctx: &Context,
    input: CrawlDetailQuery,
) -> Result<queue::CrawlDetail, CallError> {
    queue::detail(&ctx.db, input.id, input.page, input.failed)
        .await?
        .ok_or_else(|| CallError::not_found("这次整理已被删除"))
}
pub(super) async fn crawl_usage(
    ctx: &Context,
    input: CrawlId,
) -> Result<stats::TokenTotals, CallError> {
    Ok(stats::crawl_tokens(&ctx.db, input.id).await?)
}
pub(super) async fn pause_crawl(ctx: &Context, input: CrawlId) -> Result<(), CallError> {
    queue::control(&ctx.db, input.id, "pause", now()).await?;
    ctx.crawler.wake();
    Ok(())
}
pub(super) async fn resume_crawl(ctx: &Context, input: CrawlId) -> Result<(), CallError> {
    let id = sqlx::query_scalar!("SELECT model_config_id FROM crawl WHERE id = ?", input.id)
        .fetch_optional(&ctx.db)
        .await?
        .flatten();
    let enrichment = sqlx::query_scalar!(
        r#"SELECT EXISTS (
            SELECT 1
            FROM crawl_page
            WHERE crawl_id = ? AND candidate_id IS NOT NULL
            AND work_state != 'done'
        ) AS "present!: bool""#,
        input.id
    )
    .fetch_one(&ctx.db)
    .await?;
    let ordinary = sqlx::query_scalar!(
        r#"SELECT EXISTS (
            SELECT 1
            FROM crawl_page
            WHERE crawl_id = ? AND candidate_id IS NULL AND work_state != 'done'
        ) AS "present!: bool""#,
        input.id
    )
    .fetch_one(&ctx.db)
    .await?;
    if ordinary {
        crawl_ready(ctx, id).await?;
    }
    if enrichment {
        crawl_ready(ctx, None).await?;
    }
    queue::control(&ctx.db, input.id, "resume", now()).await?;
    ctx.crawler.restart(input.id);
    Ok(())
}
pub(super) async fn cancel_crawl(ctx: &Context, input: CrawlId) -> Result<(), CallError> {
    queue::control(&ctx.db, input.id, "cancel", now()).await?;
    ctx.crawler.wake();
    Ok(())
}
pub(super) async fn retry_crawl(ctx: &Context, input: CrawlRetry) -> Result<u64, CallError> {
    let id = sqlx::query_scalar!("SELECT model_config_id FROM crawl WHERE id = ?", input.id)
        .fetch_optional(&ctx.db)
        .await?
        .flatten();
    // A retry reads the failed pages again and sets the crawl going, so the pages still to read
    // are read too; with no failed page to read again, it leaves the crawl as it is.
    let pages = sqlx::query!(
        r#"SELECT EXISTS (
            SELECT 1
            FROM crawl_page
            WHERE crawl_id = ?1 AND (?2 IS NULL OR id = ?2) AND work_state = 'done'
            AND status != 'success' AND (candidate_id IS NULL
            OR candidate_id IN (SELECT id FROM candidate WHERE review = 'pending'))
        ) AS "retried!: bool", EXISTS (
            SELECT 1
            FROM crawl_page
            WHERE crawl_id = ?1 AND candidate_id IS NULL AND (work_state != 'done'
            OR ((?2 IS NULL OR id = ?2) AND status != 'success'))
        ) AS "ordinary!: bool", EXISTS (
            SELECT 1
            FROM crawl_page
            WHERE crawl_id = ?1 AND candidate_id IS NOT NULL AND (work_state != 'done'
            OR ((?2 IS NULL OR id = ?2) AND status != 'success'
            AND candidate_id IN (SELECT id FROM candidate WHERE review = 'pending')))
        ) AS "enrichment!: bool""#,
        input.id,
        input.page
    )
    .fetch_one(&ctx.db)
    .await?;
    if !pages.retried {
        return Ok(0);
    }
    if pages.ordinary {
        crawl_ready(ctx, id).await?;
    }
    if pages.enrichment {
        crawl_ready(ctx, None).await?;
    }
    let n = queue::retry(&ctx.db, input.id, input.page, now()).await?;
    ctx.crawler.restart(input.id);
    Ok(n)
}
pub(super) async fn delete_crawl(ctx: &Context, input: CrawlId) -> Result<(), CallError> {
    queue::delete(&ctx.db, input.id).await?;
    ctx.crawler.wake();
    Ok(())
}
pub(super) async fn preview_enrich(
    ctx: &Context,
    input: CandidateIds,
) -> Result<crawl::enrich::EnrichPlan, CallError> {
    Ok(crawl::enrich::preview(&ctx.db, &input.ids).await?)
}
pub(super) async fn enrich_candidates(
    ctx: &Context,
    input: CandidateIds,
) -> Result<crawl::enrich::EnrichPlan, CallError> {
    crawl_ready(ctx, None).await?;
    let (plan, reopened) = crawl::enrich::start(&ctx.db, &input.ids, now()).await?;
    for crawl in reopened {
        ctx.crawler.restart(crawl);
    }
    ctx.crawler.wake();
    Ok(plan)
}
pub(super) async fn preview_enrich_mentors(
    ctx: &Context,
    input: crate::MentorIds,
) -> Result<crawl::enrich::EnrichPlan, CallError> {
    Ok(crawl::enrich::preview_mentors(&ctx.db, &input.ids).await?)
}
pub(super) async fn enrich_mentors(
    ctx: &Context,
    input: crate::MentorIds,
) -> Result<crawl::enrich::MentorEnrichment, CallError> {
    crawl_ready(ctx, None).await?;
    let started = crawl::enrich::start_mentors(&ctx.db, &input.ids, now()).await?;
    ctx.crawler.wake();
    Ok(started)
}
pub(super) async fn list_candidates(
    ctx: &Context,
    input: review::CandidateQuery,
) -> Result<review::CandidatePage, CallError> {
    Ok(review::list(&ctx.db, &input).await?)
}
pub(super) async fn list_candidate_ids(
    ctx: &Context,
    input: review::CandidateSelection,
) -> Result<Vec<i64>, CallError> {
    Ok(review::ids(&ctx.db, &input).await?)
}
pub(super) async fn get_candidate(
    ctx: &Context,
    input: CrawlId,
) -> Result<review::CandidateDetail, CallError> {
    review::detail(&ctx.db, input.id)
        .await?
        .ok_or_else(|| CallError::not_found("这位候选已被删除"))
}
pub(super) async fn preview_candidates(
    ctx: &Context,
    input: review::ReviewInput,
) -> Result<mentor::intake::Preview, CallError> {
    Ok(review::preview(&ctx.db, &input).await?)
}
pub(super) async fn accept_candidates(
    ctx: &Context,
    input: review::ReviewInput,
) -> Result<review::Reviewed, CallError> {
    Ok(review::accept(&ctx.db, &input, now()).await?)
}
pub(super) async fn reject_candidates(
    ctx: &Context,
    input: CandidateIds,
) -> Result<u64, CallError> {
    Ok(review::reject(&ctx.db, &input.ids).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[sqlx::test(migrator = "db::MIGRATOR")]
    async fn starting_uses_the_selected_model(db: SqlitePool) {
        let mut ids = Vec::new();
        for name in ["first", "second"] {
            let id = sqlx::query_scalar!(
                r#"INSERT INTO model_config (provider, base_url, model, api_key, agreed,
                    created_at)
                VALUES ('custom', 'http://127.0.0.1:1', ?, 'fixture', 1, 1)
                RETURNING id"#,
                name
            )
            .fetch_one(&db)
            .await
            .unwrap();
            ids.push(id);
        }
        let ctx = Context::new(
            db.clone(),
            net::Net::new(vec![]).unwrap(),
            std::path::Path::new(""),
        );
        sqlx::query!("UPDATE profile SET default_model_config_id = ?", ids[1])
            .execute(&db)
            .await
            .unwrap();
        let started = start_crawl(
            &ctx,
            CrawlStart {
                urls: "https://school.example.edu/".into(),
                model_id: ids[0],
            },
        )
        .await
        .unwrap();
        assert_eq!(
            sqlx::query_scalar!("SELECT model_config_id FROM crawl WHERE id = ?", started.id)
                .fetch_one(&db)
                .await
                .unwrap(),
            Some(ids[0])
        );
    }
    #[sqlx::test(migrator = "db::MIGRATOR")]
    async fn enrichment_requires_current_default_consent_instead_of_original_crawl_model(
        db: SqlitePool,
    ) {
        let first = sqlx::query_scalar::<_, i64>(
            r#"INSERT INTO model_config (provider, base_url, model, api_key, agreed,
                created_at)
            VALUES ('custom', 'http://127.0.0.1:1', 'original', 'fixture', 1, 1)
            RETURNING id"#,
        )
        .fetch_one(&db)
        .await
        .unwrap();
        let default = sqlx::query_scalar::<_, i64>(
            r#"INSERT INTO model_config (provider, base_url, model, api_key, agreed,
                created_at)
            VALUES ('custom', 'http://127.0.0.1:2', 'default', 'fixture', 0, 1)
            RETURNING id"#,
        )
        .fetch_one(&db)
        .await
        .unwrap();
        sqlx::query("UPDATE profile SET default_model_config_id=?")
            .bind(default)
            .execute(&db)
            .await
            .unwrap();
        let crawl = sqlx::query_scalar::<_, i64>(
            r#"INSERT INTO crawl (kind, start_urls, model_config_id, state, created_at,
                finished_at)
            VALUES ('site', '[]', ?, 'done', 1, 1)
            RETURNING id"#,
        )
        .bind(first)
        .fetch_one(&db)
        .await
        .unwrap();
        let id = sqlx::query_scalar::<_, i64>(
            r#"INSERT INTO candidate (crawl_id, profile_url, review, enrich, name)
            VALUES (?, 'https://school.example.edu/p', 'pending', 'none', '张三')
            RETURNING id"#,
        )
        .bind(crawl)
        .fetch_one(&db)
        .await
        .unwrap();
        let ctx = Context::new(
            db.clone(),
            net::Net::new(vec![]).unwrap(),
            std::path::Path::new(""),
        );
        let error = enrich_candidates(&ctx, CandidateIds { ids: vec![id] })
            .await
            .unwrap_err();
        assert!(matches!(error.code, ErrorCode::NeedsAgreement));
        assert_eq!(
            preview_enrich(&ctx, CandidateIds { ids: vec![id] })
                .await
                .unwrap()
                .ids,
            vec![id]
        );
        model::agree(
            &db,
            default,
            &model::Source::check("custom", None, "http://127.0.0.1:2").unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(
            enrich_candidates(&ctx, CandidateIds { ids: vec![id] })
                .await
                .unwrap()
                .ids,
            vec![id]
        );
        let claim = queue::next(&db).await.unwrap().unwrap();
        assert_eq!(claim.model_id, None);
        assert_eq!(
            model::selected(&db, claim.model_id, model::Feature::Crawl)
                .await
                .unwrap()
                .unwrap()
                .id,
            default
        );
        let summary = queue::work(&db, 0, i64::MAX).await.unwrap();
        assert_eq!(summary[0].enrich_remaining, 1);
        assert_eq!(summary[0].stage, queue::CrawlStage::Enrich);
        assert_eq!(summary[0].model_ids.0, vec![default]);
    }
    #[sqlx::test(migrator = "db::MIGRATOR")]
    async fn going_on_and_retrying_need_the_models_of_the_pages_they_read(db: SqlitePool) {
        let mut ids = Vec::new();
        for (address, agreed) in [("http://127.0.0.1:1", true), ("http://127.0.0.1:2", false)] {
            let id = sqlx::query_scalar::<_, i64>(
                r#"INSERT INTO model_config (provider, base_url, model, api_key, agreed,
                    created_at)
                VALUES ('custom', ?, 'fixture', 'fixture', ?, 1)
                RETURNING id"#,
            )
            .bind(address)
            .bind(agreed)
            .fetch_one(&db)
            .await
            .unwrap();
            ids.push(id);
        }
        let crawl = sqlx::query_scalar::<_, i64>(
            r#"INSERT INTO crawl (kind, start_urls, model_config_id, state, created_at)
            VALUES ('site', '[]', ?, 'paused', 1)
            RETURNING id"#,
        )
        .bind(ids[0])
        .fetch_one(&db)
        .await
        .unwrap();
        let candidate = sqlx::query_scalar::<_, i64>(
            r#"INSERT INTO candidate (crawl_id, profile_url, review, enrich, name)
            VALUES (?, 'https://school.example.edu/p', 'pending', 'done', '张三')
            RETURNING id"#,
        )
        .bind(crawl)
        .fetch_one(&db)
        .await
        .unwrap();
        // A list page still to read, on the crawl's model, and a page whose enrichment failed,
        // which a retry reads on 智能整理's model.
        sqlx::query(
            r#"INSERT INTO crawl_page (crawl_id, url, kind, candidate_id, mode, status, error,
                work_state)
            VALUES (?1, 'https://school.example.edu/list', 'list', NULL, 'http', 'failed',
                json_object('message', '尚未读取', 'details', NULL), 'queued'),
            (?1, 'https://school.example.edu/p', 'enrich_profile', ?2, 'http', 'failed',
                json_object('message', '读取失败', 'details', NULL), 'done')"#,
        )
        .bind(crawl)
        .bind(candidate)
        .execute(&db)
        .await
        .unwrap();
        let failed =
            sqlx::query_scalar::<_, i64>("SELECT id FROM crawl_page WHERE candidate_id = ?")
                .bind(candidate)
                .fetch_one(&db)
                .await
                .unwrap();
        assert!(
            model::set_feature(&db, model::Feature::Crawl, Some(ids[1]))
                .await
                .unwrap()
        );
        let detail = queue::detail(&db, crawl, 1, false).await.unwrap().unwrap();
        assert_eq!(detail.summary.model_ids.0, vec![ids[0]]);
        let mut retried = detail.summary.retry_model_ids.0.clone();
        retried.sort();
        assert_eq!(retried, ids);
        assert_eq!(detail.summary.retryable, 1);
        let retryable = detail.pages.iter().filter(|p| p.retryable);
        assert_eq!(retryable.map(|p| p.id).collect::<Vec<_>>(), vec![failed]);
        let ctx = Context::new(
            db.clone(),
            net::Net::new(vec![]).unwrap(),
            std::path::Path::new(""),
        );
        let error = retry_crawl(
            &ctx,
            CrawlRetry {
                id: crawl,
                page: None,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(error.code, ErrorCode::NeedsAgreement));
        resume_crawl(&ctx, CrawlId { id: crawl }).await.unwrap();
        // Retrying one page goes on with the list page too, now on a model not agreed to.
        sqlx::query("UPDATE model_config SET agreed = NOT agreed")
            .execute(&db)
            .await
            .unwrap();
        let error = retry_crawl(
            &ctx,
            CrawlRetry {
                id: crawl,
                page: Some(failed),
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(error.code, ErrorCode::NeedsAgreement));
        // The pages of a candidate already reviewed are not read again: a retry reads nothing.
        sqlx::query("UPDATE candidate SET review = 'rejected'")
            .execute(&db)
            .await
            .unwrap();
        let detail = queue::detail(&db, crawl, 1, false).await.unwrap().unwrap();
        assert_eq!(detail.summary.retryable, 0);
        assert!(detail.pages.iter().all(|p| !p.retryable));
        for page in [Some(failed), None] {
            assert_eq!(
                retry_crawl(&ctx, CrawlRetry { id: crawl, page })
                    .await
                    .unwrap(),
                0
            );
        }
    }
}
