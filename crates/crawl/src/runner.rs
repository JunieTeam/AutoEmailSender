//! Background page/model work. Page results and newly discovered work commit together.
use crate::{
    Material, PageTarget, PageUrl, Scope,
    browser::{Browser, BrowserError},
    embedded, enrich,
    extract::{self, Kind, Person},
    fetch::Fetcher,
    now,
    queue::{self, Claim},
    review,
    store::{PageKind, PageMethod, PageStatus, PageStep},
};
use failure::Failure;
use mentor::Affiliation;
use model::{
    Chosen,
    background::{CONCURRENCY, Effect, LASTING, Limited, RateLimit, Rounds, Turns},
};
use serde::Serialize;
use sqlx::{SqliteConnection, SqlitePool, types::Json};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{
    sync::{Notify, watch},
    task::JoinSet,
    time::Instant,
};
use tracing::Instrument;
use ts_rs::TS;
/// Crawls share it: they can use the same model and websites.
type Cooldown = model::background::Cooldown<i64>;
const READ_FAILED: &str = "整理数据暂时无法读取，请继续或重启应用";
const SAVE_FAILED: &str = "网页资料暂时无法保存，请继续或重启应用";
const MODEL_READ_FAILED: &str = "模型配置暂时无法读取，请重试";
const CANDIDATE_READ_FAILED: &str = "候选资料暂时无法读取，请继续或重启应用";
#[derive(Clone, Default)]
pub struct Crawler {
    wake: Arc<Notify>,
    rounds: Arc<Rounds<i64>>,
    writes: db::Writes,
}
impl Crawler {
    pub fn new(writes: db::Writes) -> Self {
        Self {
            wake: Arc::default(),
            rounds: Arc::default(),
            writes,
        }
    }
    pub fn wake(&self) {
        self.wake.notify_one();
    }
    /// The user resumed or retried `crawl`.
    pub fn restart(&self, crawl: i64) {
        self.rounds.next(crawl);
        self.wake();
    }
}
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CrawlFinished {
    pub id: i64,
    pub message: String,
}
struct Pages {
    writes: db::Writes,
    http: Fetcher,
    browser: Option<Arc<dyn Browser>>,
    recognize: embedded::Recognize,
}

async fn save_material(
    db: &SqlitePool,
    c: &Claim,
    m: &Material,
    method: PageMethod,
) -> Result<(), ReadError> {
    let text = m.text();
    let reason = (m.needs_browser.as_ref()).map(|r| Json(Failure::new(browser(r))));
    let status = if reason.is_some() {
        PageStatus::NeedsBrowser
    } else {
        PageStatus::Success
    };
    let saved = sqlx::query!(
        r#"UPDATE crawl_page
        SET text = ?, mode = ?, status = ?, error = ?, fetched_at = ?
        WHERE id = ? AND claim = ? AND work_state = 'running'
        AND EXISTS (
            SELECT 1
            FROM crawl
            WHERE id = crawl_page.crawl_id AND state = 'running'
        )"#,
        text,
        method,
        status,
        reason,
        now(),
        c.id,
        c.generation
    )
    .execute(db)
    .await
    .map_err(database("save fetched page", SAVE_FAILED))?;
    if saved.rows_affected() == 0 {
        return Err(ReadError::Gone);
    }
    Ok(())
}

impl Pages {
    /// The page, and whether the browser read it. `target` becomes the address the page was
    /// reached at, where the browser opens it too.
    async fn fetch(
        &self,
        db: &SqlitePool,
        c: &Claim,
        target: &mut PageTarget,
        cooldown: &Cooldown,
    ) -> Result<(Material, bool), ReadError> {
        // A limit can begin while this page is being claimed; it then waits its turn again.
        if cooldown.until().is_some() {
            return Err(ReadError::Wait);
        }
        let round = cooldown.round(&c.crawl);
        let documents = c.kind == PageKind::EnrichChild;
        let fetched = self
            .http
            .fetch(target, documents)
            .await
            .map_err(|e| match e {
                // Like a model's rate limit: no new pages start until then, and this one goes again.
                crate::CrawlError::SlowDown(wait) => {
                    tracing::info!(wait_ms = wait.as_millis(), "website rate limited");
                    hold_back(cooldown, c.crawl, round, wait);
                    ReadError::RateLimit(
                        RateLimit {
                            wait,
                            lasting: Failure::new(format!(
                                "网站要求放慢访问已超过 {} 分钟，页面一直未能打开。\
                                可能是网站限制了你的网络，可稍后再继续，或更换网络后继续。",
                                LASTING.as_secs() / 60
                            )),
                        },
                        round,
                    )
                }
                e => ReadError::Failed(e.failure()),
            })?;
        if fetched.over_http {
            *target = target.over_http();
        }
        let mut m = fetched.material;
        if m.needs_browser.is_none()
            && let Some(step) = c.steps.last()
        {
            m.needs_browser = Some(match step.page {
                Some(_) => crate::BrowserReason::Pagination,
                None => crate::BrowserReason::Click,
            });
        }
        if let Some(reason) = &m.needs_browser {
            tracing::info!(
                url = c.url.as_str(),
                ?reason,
                available = self.browser.is_some(),
                "browser takeover requested"
            );
        }
        save_material(db, c, &m, PageMethod::Http).await?;
        if m.needs_browser.is_none() && c.steps.is_empty() {
            return Ok((m, false));
        }
        match self
            .browse(db, c, target, &m, fetched.relaxed_certificate)
            .await
        {
            None => Ok((m, false)),
            Some(rendered) => Ok((rendered?, true)),
        }
    }

    /// The page `read` over HTTP, read once more in the browser; none when there is no browser to
    /// read it in.
    async fn render_again(
        &self,
        db: &SqlitePool,
        c: &Claim,
        target: &PageTarget,
        read: &Material,
    ) -> Result<Option<Material>, ReadError> {
        if self.browser.is_some() {
            tracing::info!(
                url = c.url.as_str(),
                "nothing over HTTP, reading in the browser"
            );
        }
        self.browse(db, c, target, read, false).await.transpose()
    }

    async fn browse(
        &self,
        db: &SqlitePool,
        c: &Claim,
        target: &PageTarget,
        read: &Material,
        relaxed_certificate: bool,
    ) -> Option<Result<Material, ReadError>> {
        let browser = self.browser.as_ref()?;
        if let Err(e) = save_material(db, c, read, PageMethod::Webview).await {
            return Some(Err(e));
        }
        let started = Instant::now();
        let result = Self::render(browser.as_ref(), db, c, target, relaxed_certificate).await;
        let status = match &result {
            Ok(_) => "success",
            Err(ReadError::Gone) => "canceled",
            Err(_) => "failed",
        };
        tracing::info!(
            url = c.url.as_str(),
            method = "browser",
            result = status,
            elapsed_ms = started.elapsed().as_millis(),
            "page fetched"
        );
        Some(result)
    }

    async fn render(
        browser: &dyn Browser,
        db: &SqlitePool,
        c: &Claim,
        target: &PageTarget,
        relaxed_certificate: bool,
    ) -> Result<Material, ReadError> {
        let render = browser.render(target, &c.steps);
        let rendered = tokio::select! {
            result = render => result.map_err(|e| {
                let category = match &e {
                    BrowserError::TimedOut if relaxed_certificate => "certificate_timeout",
                    BrowserError::TimedOut => "timeout",
                    BrowserError::Failed(_) => "browser_error",
                };
                tracing::info!(url = c.url.as_str(), category, "browser read failed");
                ReadError::Failed(match e {
                    BrowserError::TimedOut if relaxed_certificate => {
                        Failure::new("网站的证书有问题，浏览器无法打开")
                    }
                    e => e.failure(),
                })
            })?,
            error = invalidated(db, c) => return Err(error),
        };
        let url = target.redirect(target.url(), &rendered.url).map_err(|e| {
            tracing::info!(
                error = %e,
                "browser result URL rejected"
            );
            ReadError::Failed(e.failure())
        })?;
        if rendered.html.len() > crate::fetch::MAX_PAGE_BYTES {
            tracing::info!(reason = "too_large", "browser content rejected");
            return Err(ReadError::Failed(crate::CrawlError::TooLarge.failure()));
        }
        let span = tracing::Span::current();
        let mut material = tokio::task::spawn_blocking(move || {
            span.in_scope(|| crate::parse_rendered(&url, &rendered.html))
        })
        .await
        .map_err(|e| {
            tracing::info!(reason = "parse_worker_failed", "browser content rejected");
            ReadError::Failed(crate::CrawlError::Worker(e).failure())
        })?;
        // Interactive pagination remains present after rendering, and is now replayable.
        if material.needs_browser == Some(crate::BrowserReason::Pagination)
            && !material.pagination.buttons.is_empty()
        {
            material.needs_browser = None;
        }
        if let Some(reason) = &material.needs_browser {
            tracing::info!(?reason, "browser content still unavailable");
            return Err(ReadError::Failed(Failure::new(format!(
                "浏览器读取后仍未取得页面内容：{}",
                self::browser(reason)
            ))));
        }
        let attached = material.images.as_ref().is_some_and(|i| !i.list.is_empty())
            || !material.documents.is_empty();
        if material.units.is_empty() && !(targeted(c.kind) && attached) {
            tracing::info!(reason = "empty_page", "browser content still unavailable");
            return Err(ReadError::Failed(Failure::new(
                "浏览器读取后仍未取得页面内容，网站可能尚未通过验证",
            )));
        }
        save_material(db, c, &material, PageMethod::Webview).await?;
        Ok(material)
    }
}

struct Page {
    material: Material,
    kind: PageKind,
    people: Vec<Person>,
    next: Vec<PageUrl>,
    /// The labels of controls to click on the page as it opens, each showing a part to read.
    clicks: Vec<String>,
    /// The page proved to be its one person's own, so where it ended up identifies them.
    own: bool,
    /// The school and college the page says it belongs to.
    unit: Option<Affiliation>,
    /// The school and college its people's units were filled in at, which the user may have
    /// changed while it was being read.
    filled: Option<Affiliation>,
}
impl Page {
    fn nothing(material: Material, kind: PageKind) -> Self {
        Page {
            material,
            kind,
            people: vec![],
            next: vec![],
            clicks: vec![],
            own: false,
            unit: None,
            filled: None,
        }
    }
    /// Gives no one and leads nowhere.
    fn is_empty(&self) -> bool {
        let p = &self.material.pagination;
        self.people.is_empty()
            && self.next.is_empty()
            && self.clicks.is_empty()
            && p.urls.is_empty()
            && p.buttons.is_empty()
    }
}
enum Outcome {
    Done(Box<Page>),
    Failed(Failure),
    Mismatched {
        reason: Failure,
        reached: PageUrl,
    },
    Pause(Failure, Option<Chosen>),
    /// With the round of the limited request.
    RateLimit(RateLimit, Chosen, u64),
    /// A rate limit began while the page was being claimed.
    Wait,
    Gone,
}

#[derive(Debug)]
enum ReadError {
    Failed(Failure),
    Mismatched {
        reason: Failure,
        /// Boxed so that every read's result stays small.
        reached: Box<PageUrl>,
    },
    Pause(Failure),
    Database {
        source: sqlx::Error,
        operation: &'static str,
        reason: &'static str,
    },
    RateLimit(RateLimit, u64),
    Wait,
    Gone,
}

impl ReadError {
    /// How much of the crawl it holds back: a pause all of it, a rate limit its calls for a while,
    /// anything else this page alone, and a call given up because another failed nothing.
    fn reach(&self) -> u8 {
        match self {
            Self::Pause(_) | Self::Database { .. } => 3,
            Self::RateLimit(..) => 2,
            Self::Failed(_) | Self::Mismatched { .. } | Self::Gone => 1,
            Self::Wait => 0,
        }
    }
}

/// Holds the crawls back from the moment a call meets a limit, not once its page is done: the
/// page's other calls may take a while yet. A limit met in a round the user has since gone on
/// from holds nothing back: the new round's own calls find out whether it still holds.
fn hold_back(cooldown: &Cooldown, crawl: i64, round: u64, wait: Duration) {
    if round == cooldown.round(&crawl) {
        cooldown.defer(crawl, round, wait);
    }
}

fn database(
    operation: &'static str,
    reason: &'static str,
) -> impl FnOnce(sqlx::Error) -> ReadError {
    move |source| ReadError::Database {
        source,
        operation,
        reason,
    }
}

struct Reader<'a> {
    writes: &'a db::Writes,
    db: &'a SqlitePool,
    net: &'a net::Net,
    turns: &'a Turns,
    claim: &'a Claim,
    chosen: &'a Chosen,
    cooldown: &'a Cooldown,
    /// Set once one of the page's calls has failed: the page is read again later, so its calls not
    /// sent yet are not sent.
    failed: watch::Sender<bool>,
}
impl Reader<'_> {
    async fn ask<T: extract::Output>(
        &self,
        step: &'static str,
        system: &str,
        data: serde_json::Value,
    ) -> Result<T, ReadError> {
        let mut rules = system.to_owned();
        for attempt in 0..2 {
            let turn = loop {
                let mut failed = self.failed.subscribe();
                let turn = tokio::select! {
                    turn = self.cooldown.acquire(self.turns, self.chosen) => turn,
                    _ = failed.wait_for(|&failed| failed) => return Err(ReadError::Wait),
                };
                let mut conn = self
                    .db
                    .acquire()
                    .await
                    .map_err(database("acquire model claim connection", READ_FAILED))?;
                if !queue::valid(&mut conn, self.claim)
                    .await
                    .map_err(database("check model claim", READ_FAILED))?
                {
                    return Err(ReadError::Gone);
                }
                let current =
                    model::selected(&mut *conn, self.claim.model_id, model::Feature::Crawl)
                        .await
                        .map_err(database("read model configuration", MODEL_READ_FAILED))?;
                if !current.as_ref().is_some_and(|m| {
                    m.agreed
                        && m.provider == self.chosen.provider
                        && model::background::same_config(m, self.chosen)
                }) {
                    return Err(ReadError::Gone);
                }
                // Another part may have failed while this one checked.
                if *self.failed.borrow() {
                    return Err(ReadError::Wait);
                }
                if self.cooldown.until().is_none() {
                    break turn;
                }
            };
            let round = self.cooldown.round(&self.claim.crawl);
            let started = Instant::now();
            let answer = model::ask(self.net, &turn, &rules, &data.to_string(), true)
                .await
                .map_err(|e| {
                    let category = match &e {
                        model::ModelError::MissingKey => "missing_key",
                        model::ModelError::Unreachable { .. } => "unreachable",
                        model::ModelError::TimedOut { .. } => "timeout",
                        model::ModelError::Refused { .. } => "refused",
                        model::ModelError::Failed(_) => "stream_failed",
                        model::ModelError::NoAnswer { .. } => "no_answer",
                        model::ModelError::Broken(_) => "invalid_response",
                    };
                    let status = match &e {
                        model::ModelError::Refused { status, .. } => Some(*status),
                        _ => None,
                    };
                    let (error, diagnostic) = (e.to_string(), e.diagnostic());
                    let failure = model::background::classify(&e, &self.chosen.access);
                    let action = match &failure {
                        Effect::Pause(_) => "pause",
                        Effect::RateLimit(_) => "cooldown",
                        Effect::Failed(_) => "fail_page",
                    };
                    tracing::info!(
                        url = self.claim.url.as_str(),
                        step,
                        attempt,
                        category,
                        status,
                        %error,
                        diagnostic,
                        action,
                        elapsed_ms = started.elapsed().as_millis(),
                        "model call failed"
                    );
                    match failure {
                        Effect::Pause(r) => ReadError::Pause(r),
                        Effect::RateLimit(limit) => {
                            tracing::info!(wait_ms = limit.wait.as_millis(), "model cooldown");
                            hold_back(self.cooldown, self.claim.crawl, round, limit.wait);
                            ReadError::RateLimit(limit, round)
                        }
                        Effect::Failed(r) => ReadError::Failed(r),
                    }
                })?;
            tracing::info!(
                url = self.claim.url.as_str(),
                step,
                attempt,
                usage_known = answer.usage.is_some(),
                input_tokens = answer.usage.map(|u| u.input),
                cached_tokens = answer.usage.map(|u| u.cached),
                output_tokens = answer.usage.map(|u| u.output),
                elapsed_ms = started.elapsed().as_millis(),
                "model call completed"
            );
            if let Some(usage) = answer.usage {
                self.writes
                    .retry("record crawl usage", || {
                        model::usage::record_crawl(
                            self.db,
                            self.chosen,
                            self.claim.crawl,
                            usage,
                            answer.sent_at,
                        )
                    })
                    .await;
            }
            if answer.finish_reason == Some(model::Finish::Length) {
                tracing::info!(step, reason = "output_truncated", "model output rejected");
                return Err(ReadError::Failed(Failure::new(model::CUT_OFF)));
            }
            match extract::parse(&answer.text) {
                Ok(v) => return Ok(v),
                Err(reason) if attempt == 0 => {
                    tracing::info!(step, reason = "invalid_fields", "retry model output");
                    rules =
                        format!("{system}\n上次输出未通过字段校验：{reason}。请重新输出完整 JSON。")
                }
                Err(reason) => {
                    tracing::info!(step, reason = "invalid_fields", "model output rejected");
                    return Err(ReadError::Failed(Failure::new(format!(
                        "模型返回的资料不合格：{reason}。请重试或换一个模型。"
                    ))));
                }
            }
        }
        unreachable!("both attempts return")
    }
}
#[tracing::instrument(name = "crawl", parent = None, skip_all, fields(id = c.crawl))]
async fn process(
    db: &SqlitePool,
    net: &net::Net,
    fetcher: &Pages,
    turns: &Turns,
    c: &Claim,
    cooldown: &Cooldown,
) -> Result<Outcome, sqlx::Error> {
    tracing::info!(
        url = c.url.as_str(),
        kind = ?c.kind,
        "page started"
    );
    let Some(chosen) = model::selected(db, c.model_id, model::Feature::Crawl).await? else {
        tracing::info!(reason = "missing_model", "crawl cannot continue");
        return Ok(Outcome::Pause(
            Failure::new("尚未添加模型，请在“设置 › AI › 模型”中添加后继续"),
            None,
        ));
    };
    if !chosen.agreed {
        tracing::info!(reason = "model_consent_required", "crawl cannot continue");
        return Ok(Outcome::Pause(
            Failure::new("模型配置需要确认，请确认网页内容将发往的服务商后继续"),
            Some(chosen),
        ));
    }
    Ok(
        match read(db, net, fetcher, turns, c, &chosen, cooldown).await {
            Ok(page) => Outcome::Done(Box::new(page)),
            Err(ReadError::Pause(reason)) => Outcome::Pause(reason, Some(chosen)),
            Err(ReadError::Database {
                source,
                operation,
                reason,
            }) => {
                tracing::error!(
                    operation,
                    error = %source,
                    "crawl database"
                );
                Outcome::Pause(Failure::new(reason), Some(chosen))
            }
            Err(ReadError::Failed(reason)) => Outcome::Failed(reason),
            Err(ReadError::Mismatched { reason, reached }) => Outcome::Mismatched {
                reason,
                reached: *reached,
            },
            Err(ReadError::RateLimit(limit, round)) => Outcome::RateLimit(limit, chosen, round),
            Err(ReadError::Wait) => Outcome::Wait,
            Err(ReadError::Gone) => Outcome::Gone,
        },
    )
}

async fn read(
    db: &SqlitePool,
    net: &net::Net,
    fetcher: &Pages,
    turns: &Turns,
    c: &Claim,
    chosen: &Chosen,
    cooldown: &Cooldown,
) -> Result<Page, ReadError> {
    let scope = Scope::new(&c.scope);
    // External targets can only have been persisted from Scope::linked_profile.
    let mut target = if c.kind == PageKind::Profile {
        PageTarget { url: c.url.clone() }
    } else {
        scope
            .page(c.url.clone())
            .map_err(|e| ReadError::Failed(e.failure()))?
    };
    let (mut m, mut rendered) = fetcher.fetch(db, c, &mut target, cooldown).await?;
    let reader = Reader {
        writes: &fetcher.writes,
        db,
        net,
        turns,
        claim: c,
        chosen,
        cooldown,
        failed: watch::Sender::new(false),
    };
    let targeted = targeted(c.kind);
    // A page that shows nothing over HTTP may fill in once its scripts have run.
    if !rendered
        && m.units.is_empty()
        && m.needs_browser.is_none()
        && let Some(material) = fetcher.render_again(db, c, &target, &m).await?
    {
        (m, rendered) = (material, true);
    }
    let blocked = matches!(
        m.needs_browser,
        Some(crate::BrowserReason::Challenge | crate::BrowserReason::JavaScript)
    );
    if blocked || (m.units.is_empty() && !targeted) {
        return Ok(Page::nothing(
            m,
            if targeted { c.kind } else { PageKind::Entry },
        ));
    }
    let unit = crawl_unit(db, c.crawl)
        .await
        .map_err(database("read crawl unit", READ_FAILED))?;
    if targeted {
        return read_for_candidate(
            &reader,
            fetcher,
            &scope,
            &target,
            m,
            rendered,
            unit.as_ref(),
        )
        .await;
    }
    let page = interpret(&reader, &scope, m, unit.as_ref()).await?;
    // A list that loads its teachers with scripts gives none over HTTP: read it in the browser,
    // which fails the page when it shows nothing either.
    if !rendered
        && page.is_empty()
        && let Some(material) = fetcher.render_again(db, c, &target, &page.material).await?
    {
        return interpret(&reader, &scope, material, unit.as_ref()).await;
    }
    Ok(page)
}

/// The school and college a crawl from a website found its pages to belong to.
async fn crawl_unit(
    db: impl sqlx::SqliteExecutor<'_>,
    crawl: i64,
) -> Result<Option<Affiliation>, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT university, school FROM crawl WHERE id = ? AND kind = 'site'"#,
        crawl
    )
    .fetch_optional(db)
    .await?;
    Ok(row.and_then(|r| {
        Some(Affiliation {
            university: Some(r.university?),
            school: r.school,
            ..Default::default()
        })
    }))
}

/// What a page found while looking for lists is, and what it gives.
async fn interpret(
    reader: &Reader<'_>,
    scope: &Scope,
    m: Material,
    unit: Option<&Affiliation>,
) -> Result<Page, ReadError> {
    let c = reader.claim;
    // Clicks are replayed on the page as it opens, so only that page offers its controls.
    let controls: &[String] = if c.steps.is_empty() { &m.controls } else { &[] };
    let route = reader
        .ask::<extract::Route>(
            "route",
            extract::ROUTE,
            extract::routing(&m, scope, controls),
        )
        .await?;
    let found = route.affiliation(&m);
    // The crawl's way of writing its school and college, once one page has named them.
    let unit = unit.or(found.as_ref());
    let mut next: Vec<PageUrl> = Vec::new();
    let mut clicks: Vec<String> = Vec::new();
    if !matches!(route.kind, Kind::Profile) {
        let links = extract::links(&m);
        for n in route.next.iter().filter_map(|id| id.checked_sub(1)) {
            if let Some((_, url)) = links.get(n) {
                if scope.page((*url).clone()).is_ok()
                    && !m.pagination.urls.contains(url)
                    && !url.is_normalization_of(&c.url)
                    && !url.is_normalization_of(&m.url)
                    && !next.contains(*url)
                {
                    next.push((*url).clone());
                }
            } else if let Some(control) = controls.get(n - links.len())
                && !clicks.contains(control)
            {
                clicks.push(control.clone());
            }
        }
    }
    let people = match route.kind {
        // Routing sees only a page's edges and links, which do not show names written as plain
        // text: an entry can list teachers too.
        Kind::List | Kind::Entry => {
            let parts = extract::segments(&m).into_iter().map(|range| async {
                let part = reader
                    .ask::<extract::People>("list", extract::LIST, extract::data(&m, range))
                    .await;
                if part.is_err() {
                    reader.failed.send_replace(true);
                }
                part
            });
            // Each request sent runs to its end, so that what it used is recorded when another
            // fails.
            let mut all = Vec::new();
            let mut failure: Option<ReadError> = None;
            for part in futures_util::future::join_all(parts).await {
                match part {
                    Ok(part) => all.extend(part.people),
                    Err(e) if failure.as_ref().is_none_or(|f| e.reach() > f.reach()) => {
                        failure = Some(e);
                    }
                    Err(_) => {}
                }
            }
            if let Some(e) = failure {
                return Err(e);
            }
            extract::listed(&m, all, scope, unit).map_err(|reason| {
                tracing::info!(reason = "invalid_people", "page extraction failed");
                ReadError::Failed(Failure::new(reason))
            })?
        }
        Kind::Profile => {
            let p = reader
                .ask::<extract::Profile>(
                    "profile",
                    extract::PROFILE,
                    extract::data(&m, 0..m.units.len()),
                )
                .await?;
            let mut p = extract::profile(&m, p.name, p.details).map_err(|reason| {
                tracing::info!(reason = "invalid_profile", "page extraction failed");
                ReadError::Failed(Failure::new(reason))
            })?;
            if let Some(unit) = unit {
                extract::fill(&mut p.fields.affiliations, unit);
            }
            p.profile = Some(c.url.clone());
            vec![p]
        }
        Kind::Other => vec![],
    };
    let kind = match route.kind {
        Kind::Profile => PageKind::Profile,
        Kind::List => PageKind::List,
        Kind::Entry if !people.is_empty() => PageKind::List,
        Kind::Entry | Kind::Other => PageKind::Entry,
    };
    Ok(Page {
        material: m,
        kind,
        people,
        next,
        clicks,
        own: kind == PageKind::Profile,
        filled: unit.cloned(),
        unit: found,
    })
}

/// Whether the page is read for one candidate, which it may give everything in its images and PDFs.
fn targeted(kind: PageKind) -> bool {
    matches!(
        kind,
        PageKind::Profile | PageKind::EnrichProfile | PageKind::EnrichChild
    )
}

/// Settles once the claim is no longer this worker's, as after a pause; a long read stops then.
async fn invalidated(db: &SqlitePool, c: &Claim) -> ReadError {
    loop {
        let valid = async {
            let mut conn = db
                .acquire()
                .await
                .map_err(database("acquire claim check connection", READ_FAILED))?;
            queue::valid(&mut conn, c)
                .await
                .map_err(database("check claim", READ_FAILED))
        };
        match valid.await {
            Ok(true) => tokio::time::sleep(Duration::from_millis(500)).await,
            Ok(false) => return ReadError::Gone,
            Err(error) => return error,
        }
    }
}

/// Reads a profile page or enrichment page for the one candidate it was queued for, reached at `at`
/// and `rendered` if the browser read it.
async fn read_for_candidate(
    reader: &Reader<'_>,
    pages: &Pages,
    scope: &Scope,
    at: &PageTarget,
    m: Material,
    rendered: bool,
    unit: Option<&Affiliation>,
) -> Result<Page, ReadError> {
    let c = reader.claim;
    let mut conn = reader
        .db
        .acquire()
        .await
        .map_err(database("read page candidate", CANDIDATE_READ_FAILED))?;
    let id = match c.candidate_id {
        Some(id) => id,
        None => target(&mut conn, c.id)
            .await
            .map_err(database("read page candidate", CANDIDATE_READ_FAILED))?
            .ok_or(ReadError::Gone)?,
    };
    let candidate = review::read(&mut conn, &[id])
        .await
        .map_err(database("read page candidate", CANDIDATE_READ_FAILED))?
        .pop()
        .ok_or(ReadError::Gone)?;
    // Enrichment stops at review; a list's profile page still adds to a reviewed candidate, which
    // awaits review again if it was taken in, and stays rejected if it was rejected.
    if c.candidate_id.is_some() && candidate.review != "pending" {
        return Err(ReadError::Gone);
    }
    drop(conn);
    let target = candidate.fields;
    let names: Vec<_> = std::iter::once(target.name.as_str())
        .chain(target.aliases.iter().map(String::as_str))
        .collect();
    // A page that fills in its teacher with scripts shows only its template over HTTP, as
    // cta.jxufe.edu.cn writes `{{name}}` and vcc.tech's people pages ask for theirs.
    let m = if !rendered
        && c.kind != PageKind::EnrichChild
        && !extract::mentions(&m, &names)
        && let Some(material) = pages.render_again(reader.db, c, at, &m).await?
    {
        material
    } else {
        m
    };
    let files = embedded::Reader {
        fetcher: &pages.http,
        recognize: pages.recognize,
        cooldown: reader.cooldown,
        crawl: c.crawl,
    };
    let read = files.read(
        m,
        target.emails.is_empty(),
        target.emails.is_empty() || target.research.is_none(),
    );
    // Its images and PDFs can take minutes, which a pause or cancellation does not wait out.
    let m = tokio::select! {
        m = read => m.map_err(|e| ReadError::Failed(e.failure()))?,
        error = invalidated(reader.db, c) => return Err(error),
    };
    if m.units.is_empty() {
        return Ok(Page::nothing(m, c.kind));
    }
    let mut data = extract::data(&m, 0..m.units.len());
    data["target"] = serde_json::json!({
        "name": target.name,
        "aliases": target.aliases,
        "affiliations": target.affiliations
    });
    data["subpages"] = serde_json::json!(c.kind == PageKind::EnrichProfile);
    let result = reader
        .ask::<extract::Targeted>("profile", extract::TARGET, data)
        .await?;
    if result.relation == extract::Relation::Mismatched && !extract::mentions(&m, &names) {
        tracing::info!(
            url = c.url.as_str(),
            reason = "identity_mismatch",
            "page rejected"
        );
        let reason = Failure::new(format!(
            "模型判断这页是别人的或多人的页面，页面上也未出现“{}”，未补入",
            target.name
        ));
        // A subpage is only one the profile links to, never the candidate's own link.
        return Err(if c.kind == PageKind::EnrichChild {
            ReadError::Failed(reason)
        } else {
            ReadError::Mismatched {
                reason,
                reached: Box::new(m.url.clone()),
            }
        });
    }
    let mut next = Vec::new();
    if c.kind == PageKind::EnrichProfile {
        let links = extract::links(&m);
        for id in result.next {
            if let Some((_, url)) = id.checked_sub(1).and_then(|n| links.get(n))
                && scope.page((*url).clone()).is_ok()
                && !url.is_normalization_of(&c.url)
                && !url.is_normalization_of(&m.url)
                && !next.contains(*url)
            {
                next.push((*url).clone());
                if next.len() == 3 {
                    break;
                }
            }
        }
    }
    let mut person =
        extract::profile(&m, target.name.clone(), result.details).map_err(|reason| {
            tracing::info!(reason = "invalid_profile", "page extraction failed");
            ReadError::Failed(Failure::new(reason))
        })?;
    if c.kind == PageKind::Profile {
        person.profile = Some(c.url.clone());
    }
    if let Some(unit) = unit {
        extract::fill(&mut person.fields.affiliations, unit);
    }
    let own = result.relation == extract::Relation::Matched;
    // Only one person's own page vouches for every address on it. Elsewhere an address must sit
    // by their name, as on a list, and a subpage that gives more than one is not trusted for any.
    if !own || c.kind == PageKind::EnrichChild {
        let target_names: Vec<_> = names
            .iter()
            .copied()
            .chain(person.fields.aliases.iter().map(String::as_str))
            .collect();
        let page_names: Vec<_> = result
            .names
            .iter()
            .map(String::as_str)
            .chain(target_names.iter().copied())
            .collect();
        let mut owned = Vec::new();
        for email in m.units.iter().flat_map(|unit| &unit.emails) {
            if !owned.contains(&email.address)
                && crate::owns_email(&m, &page_names, &target_names, &email.address)
            {
                owned.push(email.address.clone());
            }
        }
        let trusted = c.kind != PageKind::EnrichChild || owned.len() == 1;
        person
            .fields
            .emails
            .retain(|email| trusted && owned.contains(email));
        person.fields.homepages.retain(|url| url != m.url.as_str());
    }
    person.recognized = person
        .fields
        .emails
        .iter()
        .filter(|address| m.only_from_image(address))
        .cloned()
        .collect();
    Ok(Page {
        material: m,
        kind: c.kind,
        people: vec![person],
        next,
        clicks: vec![],
        own,
        unit: None,
        filled: unit.cloned(),
    })
}
/// Whom a list's profile page is read for, or whom they have merged into since; none when it is
/// a link of nobody's.
async fn target(conn: &mut SqliteConnection, page: i64) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar!("SELECT target_id FROM crawl_page WHERE id = ?", page)
        .fetch_one(conn)
        .await
}
fn browser(reason: &crate::BrowserReason) -> &'static str {
    match reason {
        crate::BrowserReason::Challenge => "网站要求浏览器验证",
        crate::BrowserReason::JavaScript => "页面需要 JavaScript 渲染",
        crate::BrowserReason::EncryptedContent => "页面内容需要浏览器解密",
        crate::BrowserReason::Pagination => "名单需要在浏览器中点击翻页",
        crate::BrowserReason::Click => "名单的分类需要在浏览器中点开",
    }
}
#[tracing::instrument(name = "crawl", parent = None, skip_all, fields(id = c.crawl))]
async fn save(db: &SqlitePool, c: &Claim, outcome: &Outcome) -> Result<bool, sqlx::Error> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    if !queue::valid(&mut tx, c).await? {
        tracing::info!(url = c.url.as_str(), "page claim invalidated");
        return Ok(false);
    }
    match outcome {
        Outcome::Done(p) => {
            let text = p.material.text();
            let reason =
                (p.material.needs_browser.as_ref()).map(|r| Json(Failure::new(browser(r))));
            let status = if reason.is_some() {
                PageStatus::NeedsBrowser
            } else {
                PageStatus::Success
            };
            sqlx::query!(
                r#"UPDATE crawl_page
                SET work_state = 'done', kind = ?, status = ?, error = ?, text = ?,
                    fetched_at = ?
                WHERE id = ?"#,
                p.kind,
                status,
                reason,
                text,
                now(),
                c.id
            )
            .execute(&mut *tx)
            .await?;
            // The first page to name its school and college names the crawl's.
            if let Some(unit) = &p.unit {
                sqlx::query!(
                    r#"UPDATE crawl SET university = ?, school = ?
                    WHERE id = ? AND kind = 'site' AND university IS NULL AND school IS NULL"#,
                    unit.university,
                    unit.school,
                    c.crawl
                )
                .execute(&mut *tx)
                .await?;
            }
            // The user may have written the school and college otherwise while the page was read.
            let mut people = p.people.clone();
            if let Some(current) = crawl_unit(&mut *tx, c.crawl).await?
                && p.filled.as_ref() != Some(&current)
            {
                let filled = p.filled.clone().unwrap_or_default();
                for person in &mut people {
                    extract::relocate(&mut person.fields.affiliations, &filled, &current);
                }
            }
            if let Some(id) = c.candidate_id {
                if c.kind == PageKind::EnrichProfile
                    && !sqlx::query_scalar!(
                        r#"SELECT EXISTS (
                            SELECT 1
                            FROM crawl_page
                            WHERE candidate_id = ? AND kind = 'enrich_child'
                        ) AS "selected!: bool""#,
                        id
                    )
                    .fetch_one(&mut *tx)
                    .await?
                {
                    for url in &p.next {
                        enrich::enqueue(&mut tx, c.crawl, id, url, &c.scope, PageKind::EnrichChild)
                            .await?;
                    }
                }
                if c.kind == PageKind::EnrichProfile && p.own {
                    let urls = [c.url.as_str(), p.material.url.as_str()];
                    review::matched(&mut tx, id, &urls).await?;
                }
                for person in &people {
                    enrich::apply(&mut tx, id, p.material.url.as_str(), person).await?;
                }
            } else {
                for url in &p.next {
                    if c.depth < 2 {
                        queue::enqueue(
                            &mut tx,
                            c.crawl,
                            url,
                            &c.scope,
                            c.depth + 1,
                            PageKind::Entry,
                        )
                        .await?;
                    } else {
                        let target = url.as_str();
                        if !sqlx::query_scalar!(
                            r#"SELECT EXISTS (SELECT 1 FROM crawl_page WHERE crawl_id = ?
                                AND url = ?) AS "found!: bool""#,
                            c.crawl,
                            target
                        )
                        .fetch_one(&mut *tx)
                        .await?
                        {
                            queue::limit(&mut tx, c.crawl, "寻找名单已达到两层链接范围").await?;
                        }
                    }
                }
                // A part shown by a click counts as one more link away.
                for control in &p.clicks {
                    let steps = [PageStep {
                        page: None,
                        control: control.clone(),
                    }];
                    if c.depth < 2 {
                        queue::enqueue_steps(
                            &mut tx,
                            c.crawl,
                            &c.url,
                            &c.scope,
                            c.depth + 1,
                            PageKind::Entry,
                            &steps,
                        )
                        .await?;
                    } else if !sqlx::query_scalar!(
                        r#"SELECT EXISTS (SELECT 1 FROM crawl_page WHERE crawl_id = ?
                            AND url = ? AND steps = ?) AS "found!: bool""#,
                        c.crawl,
                        c.url,
                        Json(&steps[..])
                    )
                    .fetch_one(&mut *tx)
                    .await?
                    {
                        queue::limit(&mut tx, c.crawl, "寻找名单已达到两层链接范围").await?;
                    }
                }
                if p.kind == PageKind::List {
                    for url in &p.material.pagination.urls {
                        if Scope::new(&c.scope).page(url.clone()).is_ok() {
                            queue::enqueue(
                                &mut tx,
                                c.crawl,
                                url,
                                &c.scope,
                                c.depth,
                                PageKind::List,
                            )
                            .await?;
                        }
                    }
                }
                if p.kind == PageKind::List && p.material.needs_browser.is_none() {
                    let current = PageStep::page_of(&c.steps);
                    if let Some(button) = p
                        .material
                        .pagination
                        .buttons
                        .iter()
                        .filter(|b| b.page.is_none_or(|page| page > current))
                        .min_by_key(|b| b.page.unwrap_or(current + 1))
                    {
                        let mut steps = c.steps.clone();
                        steps.push(PageStep {
                            page: Some(button.page.unwrap_or(current + 1)),
                            control: button.control.clone(),
                        });
                        queue::enqueue_steps(
                            &mut tx,
                            c.crawl,
                            &c.url,
                            &c.scope,
                            c.depth,
                            PageKind::List,
                            &steps,
                        )
                        .await?;
                    }
                }
                let target = match c.kind {
                    PageKind::Profile if !p.people.is_empty() => target(&mut tx, c.id).await?,
                    _ => None,
                };
                // The transaction serializes duplicate detection with all candidate writes.
                for person in people {
                    review::save(
                        &mut tx,
                        c.crawl,
                        if p.kind == PageKind::List {
                            review::Source::List { id: c.id }
                        } else {
                            review::Source::Profile {
                                id: c.id,
                                own: p.own,
                                target,
                            }
                        },
                        p.material.url.as_str(),
                        person,
                    )
                    .await?;
                }
            }
        }
        Outcome::Failed(reason) | Outcome::Mismatched { reason, .. } => {
            sqlx::query!(
                r#"UPDATE crawl_page
                SET work_state = 'done', status = 'failed', error = ?, fetched_at = ?
                WHERE id = ?"#,
                Json(reason),
                now(),
                c.id
            )
            .execute(&mut *tx)
            .await?;
            if let Outcome::Mismatched { reached, .. } = outcome {
                let candidate = match c.candidate_id {
                    Some(id) => id,
                    None => target(&mut tx, c.id).await?.unwrap_or_else(|| {
                        unreachable!("a valid claim's page has whom it was read for")
                    }),
                };
                let urls = [c.url.as_str(), reached.as_str()];
                review::mismatched(&mut tx, candidate, &urls, now()).await?;
            }
        }
        Outcome::Pause(reason, expected) => {
            sqlx::query!(
                "UPDATE crawl_page SET work_state = 'queued' WHERE id = ?",
                c.id
            )
            .execute(&mut *tx)
            .await?;
            let current = model::selected(&mut *tx, c.model_id, model::Feature::Crawl).await?;
            if expected.as_ref().is_none_or(|e| {
                current
                    .as_ref()
                    .is_some_and(|m| model::background::same_config(m, e))
            }) {
                sqlx::query!(
                    r#"UPDATE crawl
                    SET state = 'paused', error = ?
                    WHERE id = ? AND state = 'running'"#,
                    Json(reason),
                    c.crawl
                )
                .execute(&mut *tx)
                .await?;
                sqlx::query!(
                    r#"UPDATE crawl_page
                    SET work_state = 'queued', claim = claim + 1
                    WHERE crawl_id = ? AND work_state = 'running'"#,
                    c.crawl
                )
                .execute(&mut *tx)
                .await?;
                sqlx::query!(
                    "UPDATE crawl_run SET ended_at = ? WHERE crawl_id = ? AND ended_at IS NULL",
                    now(),
                    c.crawl
                )
                .execute(&mut *tx)
                .await?;
            }
        }
        Outcome::RateLimit(..) | Outcome::Wait | Outcome::Gone => {
            sqlx::query!(
                "UPDATE crawl_page SET work_state = 'queued',claim = claim + 1 WHERE id = ?",
                c.id
            )
            .execute(&mut *tx)
            .await?;
        }
    }
    if let Some(id) = c.candidate_id {
        enrich::complete(&mut tx, id).await?;
    }
    tx.commit().await?;
    match outcome {
        Outcome::Done(p) => tracing::info!(
            url = c.url.as_str(),
            kind = ?p.kind,
            people = p.people.len(),
            next_pages = p.next.len(),
            needs_browser = p.material.needs_browser.is_some(),
            "page completed"
        ),
        Outcome::Failed(_) | Outcome::Mismatched { .. } => tracing::info!(
            url = c.url.as_str(),
            kind = ?c.kind,
            "page failed"
        ),
        Outcome::Pause(_, _) => tracing::info!("crawl paused"),
        Outcome::RateLimit(..) | Outcome::Wait => {}
        Outcome::Gone => tracing::info!(url = c.url.as_str(), "page claim invalidated"),
    }
    Ok(true)
}
async fn finish(db: &SqlitePool, at: i64) -> Result<Vec<CrawlFinished>, sqlx::Error> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let ids = sqlx::query_scalar!(
        r#"UPDATE crawl
        SET state = 'done', finished_at = ?
        WHERE state = 'running' AND NOT EXISTS (
            SELECT 1
            FROM crawl_page
            WHERE crawl_id = crawl.id AND work_state != 'done'
        )
        RETURNING id"#,
        at
    )
    .fetch_all(&mut *tx)
    .await?;
    let mut result = Vec::new();
    for &id in &ids {
        sqlx::query!(
            "UPDATE crawl_run SET ended_at = ? WHERE crawl_id = ? AND ended_at IS NULL",
            at,
            id
        )
        .execute(&mut *tx)
        .await?;
        let candidates =
            sqlx::query_scalar!("SELECT count(*) FROM candidate WHERE crawl_id = ?", id)
                .fetch_one(&mut *tx)
                .await?;
        let pages = sqlx::query!(
            r#"SELECT count(*) FILTER (WHERE status = 'success') AS "read!: i64",
                count(*) FILTER (WHERE status != 'success') AS "failed!: i64"
            FROM crawl_page
            WHERE crawl_id = ?"#,
            id
        )
        .fetch_one(&mut *tx)
        .await?;
        let failed = pages.failed;
        if candidates == 0 {
            let reason = if pages.read == 0 {
                "本次整理未能处理任何页面。请查看各页面的原因后重试。"
            } else {
                "没有在读到的网页上找到导师。请换成学院“师资队伍”“教师名录”这类页面再试；\
                要登录或只对校内开放的页面读不到。"
            };
            sqlx::query!(
                "UPDATE crawl SET state = 'failed', error = ? WHERE id = ?",
                Json(Failure::new(reason)),
                id
            )
            .execute(&mut *tx)
            .await?;
        }
        let activity = sqlx::query!(
            r#"SELECT p.candidate_id,c.enrich AS "enrich?: enrich::EnrichState"
            FROM crawl_page p LEFT JOIN candidate c ON c.id=p.candidate_id
            JOIN crawl ON crawl.id=p.crawl_id
            WHERE p.crawl_id=? AND p.work_state='done'
            AND (crawl.reported_at IS NULL OR p.fetched_at>crawl.reported_at)
            GROUP BY p.candidate_id"#,
            id
        )
        .fetch_all(&mut *tx)
        .await?;
        let mut messages = Vec::new();
        if activity.iter().any(|p| p.candidate_id.is_none()) {
            messages.push(match (candidates, pages.read) {
                (0, 0) => format!("整理未能完成：{failed} 个页面未打开，去智能整理里查看原因"),
                (0, _) => "整理没有找到导师，去智能整理里查看原因".to_owned(),
                _ => format!("整理完成：找到 {candidates} 位候选，{failed} 个页面未打开"),
            });
        }
        let enriched = activity
            .iter()
            .filter(|p| p.enrich == Some(enrich::EnrichState::Done))
            .count();
        if enriched > 0 {
            messages.push(format!("信息补全完成：{enriched} 位，去候选里查看结果"));
        }
        if !messages.is_empty() {
            sqlx::query!("UPDATE crawl SET reported_at=? WHERE id=?", at, id)
                .execute(&mut *tx)
                .await?;
        }
        result.push(CrawlFinished {
            id,
            message: messages.join("；"),
        });
    }
    let summaries = if ids.is_empty() {
        Vec::new()
    } else {
        queue::summaries(&mut tx, &ids).await?
    };
    tx.commit().await?;
    for summary in summaries {
        crate::span(summary.id).in_scope(|| {
            tracing::info!(
                state = ?summary.state,
                candidates = summary.candidates,
                pages = summary.pages,
                failed = summary.failed,
                usage_known = summary.tokens.is_some(),
                tokens = summary.tokens,
                "crawl finished"
            )
        });
    }
    Ok(result)
}
pub async fn run(
    db: SqlitePool,
    net: net::Net,
    crawler: Crawler,
    turns: Turns,
    browser: Option<Arc<dyn Browser>>,
    changed: impl Fn() + Send + 'static,
    finished: impl Fn(CrawlFinished) + Send + 'static,
) {
    let fetcher = loop {
        match Fetcher::new(&net) {
            Ok(f) => {
                break Arc::new(Pages {
                    writes: crawler.writes.clone(),
                    http: f,
                    browser,
                    recognize: ocr::recognize,
                });
            }
            Err(e) => {
                tracing::error!("create crawl fetcher: {e}");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    };
    run_with_pages(db, net, fetcher, crawler, turns, changed, finished).await;
}
async fn run_with_pages(
    db: SqlitePool,
    net: net::Net,
    fetcher: Arc<Pages>,
    crawler: Crawler,
    turns: Turns,
    changed: impl Fn() + Send + 'static,
    finished: impl Fn(CrawlFinished) + Send + 'static,
) {
    let writes = &fetcher.writes;
    writes.retry("recover crawl", || queue::recover(&db)).await;
    let mut workers = JoinSet::new();
    let mut claims = HashMap::new();
    let cooldown = Cooldown::new(crawler.rounds.clone());
    // By crawl: each has its own websites and model.
    let mut limited: HashMap<i64, Limited> = HashMap::new();
    loop {
        for result in writes.retry("finish crawl", || finish(&db, now())).await {
            changed();
            finished(result);
        }
        // Crawls that stopped running count afresh when they go on, so drop them.
        if !limited.is_empty() {
            let running = writes.retry("running crawls", || queue::running(&db)).await;
            limited.retain(|id, _| running.contains(id));
        }
        let retry_at = cooldown.until();
        let cooling = retry_at.is_some();
        if !cooling
            && workers.len() < CONCURRENCY
            && let Some(c) = writes.retry("claim crawl", || queue::next(&db)).await
        {
            let (db, net, turns, fetcher) =
                (db.clone(), net.clone(), turns.clone(), fetcher.clone());
            let claim = c.clone();
            let cooldown = cooldown.clone();
            let task = workers.spawn(async move {
                let result = process(&db, &net, &fetcher, &turns, &claim, &cooldown).await;
                (claim, result)
            });
            claims.insert(task.id(), c);
            changed();
            continue;
        }
        tokio::select! {
            Some(joined) = workers.join_next_with_id(), if !workers.is_empty() => {
                let (c, outcome) = match joined {
                    Ok((id, (c, result))) => {
                        claims.remove(&id);
                        let current = |round| round == crawler.rounds.current(&c.crawl);
                        let outcome = match result {
                            Ok(Outcome::RateLimit(limit, chosen, round))
                                if current(round)
                                    && limited.entry(c.crawl).or_default().lasting(
                                        round,
                                        Instant::now(),
                                        limit.wait,
                                        &chosen,
                                    ) =>
                            {
                                Outcome::Pause(limit.lasting, Some(chosen))
                            }
                            Ok(o) => o,
                            Err(e) => {
                                crate::span(c.crawl).in_scope(|| tracing::error!("crawl database: {e}"));
                                Outcome::Pause(
                                    Failure::new(
                                        "整理数据暂时无法保存，请继续或重启应用；仍然出错的，\
                                        可在电脑上的 设置 › 关于和日志 里反馈问题",
                                    ),
                                    None,
                                )
                            }
                        };
                        (c, outcome)
                    }
                    Err(e) => {
                        let Some(c) = claims.remove(&e.id()) else {
                            continue;
                        };
                        crate::span(c.crawl).in_scope(|| tracing::error!("crawl worker: {e}"));
                        (
                            c,
                            Outcome::Pause(
                                Failure::new(
                                    "整理意外中断，请继续或重启应用；仍然出错的，\
                                    可在电脑上的 设置 › 关于和日志 里反馈问题",
                                ),
                                None,
                            ),
                        )
                    }
                };
                let applied = writes
                    .retry("save crawl", || save(&db, &c, &outcome))
                    .instrument(crate::span(c.crawl))
                    .await;
                if applied
                    && matches!(
                        outcome,
                        Outcome::Done(_) | Outcome::Failed(_) | Outcome::Mismatched { .. }
                    )
                {
                    limited.remove(&c.crawl);
                }
                changed();
            },
            () = crawler.wake.notified() => {},
            () = async {
                if let Some(at) = retry_at {
                    tokio::time::sleep_until(at).await;
                }
            }, if cooling => {},
        }
    }
}

#[cfg(test)]
mod tests;
