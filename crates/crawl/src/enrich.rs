//! Candidate enrichment uses the crawl page queue without candidate identity merging.
use crate::{
    PageUrl,
    extract::Person,
    queue, review,
    store::{PageKind, PageStatus},
};
use failure::Failure;
use mentor::{Affiliation, export::Profile, intake::comparable};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{SqliteConnection, SqlitePool, types::Json};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(rename_all = "snake_case")]
pub enum EnrichState {
    None,
    Queued,
    Running,
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum EnrichField {
    Email,
    Alias,
    Homepage,
    Paper,
    Research,
    Affiliation,
    University,
    School,
    Department,
    Title,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct EnrichChange {
    pub field: EnrichField,
    #[ts(type = "unknown")]
    pub value: Value,
}
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Enrichment {
    pub enrich: EnrichState,
    pub enrich_changes: Vec<EnrichChange>,
    pub enrich_error: Option<Failure>,
}
#[derive(Debug, Clone, Copy, Serialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnrichSkipReason {
    MissingProfile,
    DeletedCrawl,
    Complete,
    Active,
    Reviewed,
    /// A mentor in the trash.
    Trashed,
    /// A mentor with a candidate still to be reviewed, which enriching again would duplicate.
    Pending,
    /// A mentor whose pages are all those of others enriched with them: one crawl reads a page
    /// for one candidate.
    Shared,
}
#[derive(Debug, Serialize, TS)]
pub struct EnrichSkip {
    pub reason: EnrichSkipReason,
    pub ids: Vec<i64>,
}
#[derive(Debug, Serialize, TS)]
pub struct EnrichPlan {
    pub ids: Vec<i64>,
    pub skipped: Vec<EnrichSkip>,
}
impl EnrichPlan {
    fn skip(&mut self, reason: EnrichSkipReason, id: i64) {
        if let Some(group) = self.skipped.iter_mut().find(|g| g.reason == reason) {
            group.ids.push(id);
        } else {
            self.skipped.push(EnrichSkip {
                reason,
                ids: vec![id],
            });
        }
    }
}
async fn plan(conn: &mut SqliteConnection, ids: &[i64]) -> Result<EnrichPlan, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT id AS "id!: i64",crawl_id,profile_url,review,enrich AS "enrich: EnrichState",
        coalesce(json_array_length(emails),0) AS "emails!: i64",research
        FROM candidate WHERE id IN (SELECT value FROM json_each(?)) ORDER BY id"#,
        Json(ids)
    )
    .fetch_all(conn)
    .await?;
    let mut result = EnrichPlan {
        ids: vec![],
        skipped: vec![],
    };
    for id in ids
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
    {
        let row = rows.iter().find(|r| r.id == id);
        let reason = match row {
            None => Some(EnrichSkipReason::Reviewed),
            Some(r) if r.review != "pending" => Some(EnrichSkipReason::Reviewed),
            Some(r) if matches!(r.enrich, EnrichState::Queued | EnrichState::Running) => {
                Some(EnrichSkipReason::Active)
            }
            Some(r) if r.crawl_id.is_none() => Some(EnrichSkipReason::DeletedCrawl),
            Some(r) if r.profile_url.is_none() => Some(EnrichSkipReason::MissingProfile),
            Some(r)
                if r.emails > 0 && r.research.as_deref().is_some_and(|s| !s.trim().is_empty()) =>
            {
                Some(EnrichSkipReason::Complete)
            }
            Some(_) => None,
        };
        match reason {
            Some(reason) => result.skip(reason, id),
            None => result.ids.push(id),
        }
    }
    Ok(result)
}
pub async fn preview(db: &SqlitePool, ids: &[i64]) -> Result<EnrichPlan, sqlx::Error> {
    plan(&mut *db.acquire().await?, ids).await
}
/// Queues the candidates' enrichment, also saying which ended crawls it started again.
pub async fn start(
    db: &SqlitePool,
    ids: &[i64],
    at: i64,
) -> Result<(EnrichPlan, Vec<i64>), sqlx::Error> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let result = plan(&mut tx, ids).await?;
    let mut crawls = std::collections::BTreeMap::<i64, usize>::new();
    let mut reopened = Vec::new();
    for id in &result.ids {
        let r = sqlx::query!(
            r#"SELECT crawl_id AS "crawl!: i64", profile_url AS "url!: PageUrl"
            FROM candidate
            WHERE id = ?"#,
            id
        )
        .fetch_one(&mut *tx)
        .await?;
        *crawls.entry(r.crawl).or_default() += 1;
        sqlx::query!("DELETE FROM crawl_page WHERE candidate_id=?", id)
            .execute(&mut *tx)
            .await?;
        sqlx::query!(
            "UPDATE candidate SET enrich='queued',enrich_changes='[]',enrich_error=NULL WHERE id=?",
            id
        )
        .execute(&mut *tx)
        .await?;
        enqueue(
            &mut tx,
            r.crawl,
            *id,
            &r.url,
            &r.url,
            PageKind::EnrichProfile,
        )
        .await?;
        let resumed = sqlx::query!(
            r#"UPDATE crawl
            SET state = 'running', finished_at = NULL, error = NULL
            WHERE id = ? AND state NOT IN ('running', 'paused')"#,
            r.crawl
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if resumed > 0 {
            queue::open_run(&mut tx, r.crawl, at).await?;
            reopened.push(r.crawl);
        }
    }
    tx.commit().await?;
    for (id, candidates) in crawls {
        crate::span(id).in_scope(|| tracing::info!(candidates, "enrichment queued"));
    }
    Ok((result, reopened))
}
/// Which of the mentors among `ids` can be enriched and the page to read for each: the first
/// homepage, else where they were found, that no other of them is to have read.
async fn mentor_plan(
    conn: &mut SqliteConnection,
    ids: &[i64],
) -> Result<(EnrichPlan, Vec<(Profile, PageUrl)>), sqlx::Error> {
    let pending: Vec<i64> = sqlx::query_scalar!(
        r#"SELECT mentor_id AS "mentor_id!" FROM candidate
        WHERE review = 'pending' AND mentor_id IN (SELECT value FROM json_each(?))"#,
        Json(ids)
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut plan = EnrichPlan {
        ids: vec![],
        skipped: vec![],
    };
    let mut pages = Vec::new();
    // In order of ID, as the profiles come.
    for (id, profile) in mentor::export::profiles(conn, ids).await? {
        let f = &profile.fields;
        let own: Vec<PageUrl> = f
            .homepages
            .iter()
            .chain(&profile.source_url)
            .filter_map(|url| PageUrl::parse(url).ok())
            .collect();
        let page = own
            .iter()
            .find(|url| pages.iter().all(|(_, taken)| taken != *url))
            .cloned();
        let reason = if profile.trashed {
            Some(EnrichSkipReason::Trashed)
        } else if pending.contains(&id) {
            Some(EnrichSkipReason::Pending)
        } else if !f.emails.is_empty() && f.research.is_some() && !profile.papers.is_empty() {
            Some(EnrichSkipReason::Complete)
        } else {
            None
        };
        match (reason, page) {
            (Some(reason), _) => plan.skip(reason, id),
            (None, None) if own.is_empty() => plan.skip(EnrichSkipReason::MissingProfile, id),
            (None, None) => plan.skip(EnrichSkipReason::Shared, id),
            (None, Some(page)) => {
                plan.ids.push(id);
                pages.push((profile, page));
            }
        }
    }
    Ok((plan, pages))
}
pub async fn preview_mentors(db: &SqlitePool, ids: &[i64]) -> Result<EnrichPlan, sqlx::Error> {
    Ok(mentor_plan(&mut *db.acquire().await?, ids).await?.0)
}
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MentorEnrichment {
    #[serde(flatten)]
    pub plan: EnrichPlan,
    /// Where their candidates are, when any are enriched.
    pub crawl_id: Option<i64>,
}
/// Enriches mentors already in the list through candidates copied from them, in a crawl of their
/// own, which are reviewed like any other (docs/design.md#导师). The mentor's note and tags stay out.
pub async fn start_mentors(
    db: &SqlitePool,
    ids: &[i64],
    at: i64,
) -> Result<MentorEnrichment, sqlx::Error> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let (plan, pages) = mentor_plan(&mut tx, ids).await?;
    if pages.is_empty() {
        return Ok(MentorEnrichment {
            plan,
            crawl_id: None,
        });
    }
    let crawl = sqlx::query_scalar!(
        "INSERT INTO crawl (kind, state, created_at) VALUES ('mentors', 'running', ?) RETURNING id",
        at
    )
    .fetch_one(&mut *tx)
    .await?;
    queue::open_run(&mut tx, crawl, at).await?;
    for (profile, page) in &pages {
        let f = &profile.fields;
        let (names, schools) = review::match_keys(f);
        let candidate = sqlx::query_scalar!(
            r#"INSERT INTO candidate (crawl_id, profile_url, review, mentor_id, enrich,
                enrich_changes, name, aliases, emails, affiliations, homepages, source_url,
                research, papers, match_names, match_schools)
            VALUES (?, ?, 'pending', ?, 'queued', '[]', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            RETURNING id AS "id!: i64""#,
            crawl,
            page,
            profile.id,
            f.name,
            Json(&f.aliases),
            Json(&f.emails),
            Json(&f.affiliations),
            Json(&f.homepages),
            profile.source_url,
            f.research,
            Json(&profile.papers),
            names,
            schools
        )
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query!(
            "INSERT INTO candidate_profile (candidate_id, crawl_id, url) VALUES (?, ?, ?)",
            candidate,
            crawl,
            page
        )
        .execute(&mut *tx)
        .await?;
        enqueue(
            &mut tx,
            crawl,
            candidate,
            page,
            page,
            PageKind::EnrichProfile,
        )
        .await?;
    }
    tx.commit().await?;
    crate::span(crawl)
        .in_scope(|| tracing::info!(mentors = pages.len(), "mentor enrichment queued"));
    Ok(MentorEnrichment {
        plan,
        crawl_id: Some(crawl),
    })
}
pub(crate) async fn enqueue(
    conn: &mut SqliteConnection,
    crawl: i64,
    candidate: i64,
    url: &PageUrl,
    scope: &PageUrl,
    kind: PageKind,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"INSERT OR IGNORE INTO crawl_page (crawl_id, candidate_id, url, scope_url, kind,
            mode, status, error, work_state)
        VALUES (?, ?, ?, ?, ?, 'http', 'failed', ?, 'queued')"#,
        crawl,
        candidate,
        url,
        scope,
        kind,
        Json(Failure::new(queue::NOT_READ))
    )
    .execute(conn)
    .await?;
    Ok(())
}
fn compatible(a: &Affiliation, b: &Affiliation, single: bool) -> bool {
    let pairs = [
        (&a.university, &b.university),
        (&a.school, &b.school),
        (&a.department, &b.department),
    ];
    let overlap = pairs.iter().any(|(a, b)| a.is_some() && b.is_some());
    (single || overlap)
        && pairs.iter().all(|(a, b)| {
            a.as_ref()
                .zip(b.as_ref())
                .is_none_or(|(a, b)| comparable(a) == comparable(b))
        })
}

fn change(
    changes: &mut Vec<EnrichChange>,
    field: EnrichField,
    value: impl Serialize,
) -> Result<(), sqlx::Error> {
    changes.push(EnrichChange {
        field,
        value: serde_json::to_value(value).map_err(|e| sqlx::Error::Encode(e.into()))?,
    });
    Ok(())
}
pub(crate) async fn apply(
    conn: &mut SqliteConnection,
    id: i64,
    url: &str,
    person: &Person,
) -> Result<(), sqlx::Error> {
    let mut old = review::read(conn, &[id])
        .await?
        .pop()
        .ok_or(sqlx::Error::RowNotFound)?;
    // Only a page proving it theirs, which clears that first, brings such a link back.
    let mismatched = review::mismatched_links(conn, &[id]).await?;
    let homepages: Vec<_> = person
        .fields
        .homepages
        .iter()
        .filter(|url| !mismatched.contains(url))
        .cloned()
        .collect();
    let f = &mut old.fields;
    let incoming = &person.fields;
    let changes = &mut old.enrichment.enrich_changes;
    macro_rules! fill_list {
        ($target:expr,$incoming:expr,$label:expr) => {
            if $target.is_empty() && !$incoming.is_empty() {
                $target = $incoming.clone();
                change(changes, $label, &$target)?;
            }
        };
    }
    fill_list!(f.emails, incoming.emails, EnrichField::Email);
    fill_list!(f.aliases, incoming.aliases, EnrichField::Alias);
    fill_list!(f.homepages, homepages, EnrichField::Homepage);
    fill_list!(old.papers, person.papers, EnrichField::Paper);
    if f.research.as_deref().is_none_or(|s| s.trim().is_empty()) && incoming.research.is_some() {
        f.research = incoming.research.clone();
        change(changes, EnrichField::Research, &f.research)?;
    }
    if f.affiliations.is_empty() {
        fill_list!(
            f.affiliations,
            incoming.affiliations,
            EnrichField::Affiliation
        );
    } else {
        let single = f.affiliations.len() == 1 && incoming.affiliations.len() == 1;
        let pairs: Vec<Vec<usize>> = f
            .affiliations
            .iter()
            .map(|a| {
                incoming
                    .affiliations
                    .iter()
                    .enumerate()
                    .filter(|(_, b)| compatible(a, b, single))
                    .map(|(index, _)| index)
                    .collect()
            })
            .collect();
        for (index, a) in f.affiliations.iter_mut().enumerate() {
            if let [matched] = pairs[index].as_slice()
                && pairs.iter().filter(|p| p.contains(matched)).count() == 1
            {
                let b = &incoming.affiliations[*matched];
                for (target, value, label) in [
                    (&mut a.university, &b.university, EnrichField::University),
                    (&mut a.school, &b.school, EnrichField::School),
                    (&mut a.department, &b.department, EnrichField::Department),
                    (&mut a.title, &b.title, EnrichField::Title),
                ] {
                    if target.is_none() && value.is_some() {
                        *target = value.clone();
                        change(changes, label, &target)?;
                    }
                }
            }
        }
    }
    let (names, schools) = review::match_keys(f);
    let Json(mut evidence) = sqlx::query_scalar!(
        r#"SELECT evidence AS "evidence: Json<Vec<review::Evidence>>" FROM candidate WHERE id=?"#,
        id
    )
    .fetch_one(&mut *conn)
    .await?;
    let source = review::Evidence {
        url: url.into(),
        name: incoming.name.clone(),
        emails: incoming.emails.clone(),
        recognized: person.recognized.clone(),
        research: incoming.research.clone(),
    };
    if !evidence.contains(&source) {
        evidence.push(source);
    }
    sqlx::query!(
        r#"UPDATE candidate
        SET aliases = ?, emails = ?, homepages = ?, research = ?, affiliations = ?,
            papers = ?, enrich_changes = ?, match_names = ?, match_schools = ?,
            evidence = ?
        WHERE id = ?"#,
        Json(&f.aliases),
        Json(&f.emails),
        Json(&f.homepages),
        f.research,
        Json(&f.affiliations),
        Json(&old.papers),
        Json(changes),
        names,
        schools,
        Json(evidence),
        id
    )
    .execute(conn)
    .await?;
    Ok(())
}
/// Ends the candidate's enrichment once its pages are all read: with what the first failed page
/// says, the profile's first, when nothing was found to add.
pub(crate) async fn complete(conn: &mut SqliteConnection, id: i64) -> Result<(), sqlx::Error> {
    let Some(changed) = sqlx::query_scalar!(
        r#"SELECT json_array_length(enrich_changes) > 0 AS "changed!: bool"
        FROM candidate
        WHERE id = ?1 AND review = 'pending' AND enrich IN ('queued', 'running')
        AND NOT EXISTS (
            SELECT 1
            FROM crawl_page
            WHERE candidate_id = ?1 AND work_state != 'done'
        )"#,
        id
    )
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(());
    };
    let error = if changed {
        None
    } else {
        let page = sqlx::query!(
            r#"SELECT status AS "status: PageStatus", error AS "error: Json<Failure>"
            FROM crawl_page
            WHERE candidate_id = ? AND status != 'success'
            ORDER BY status = 'needs_browser' DESC, kind = 'enrich_profile' DESC, id
            LIMIT 1"#,
            id
        )
        .fetch_optional(&mut *conn)
        .await?;
        Some(match page.and_then(|p| Some((p.status, p.error?.0))) {
            Some((status, mut failure)) => {
                let read = if status == PageStatus::NeedsBrowser {
                    "需要浏览器"
                } else {
                    "页面读取失败"
                };
                failure.message = format!("{read}：{}", failure.message);
                failure
            }
            None => Failure::new("页面上无可补充的资料"),
        })
    };
    sqlx::query!(
        "UPDATE candidate SET enrich = 'done', enrich_error = ? WHERE id = ?",
        error.map(Json),
        id
    )
    .execute(conn)
    .await?;
    Ok(())
}
/// Review invalidates work before another request can send or persist candidate data.
pub(crate) async fn retire(conn: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    let ids = sqlx::query_scalar!(
        "SELECT id FROM candidate WHERE review!='pending' AND enrich IN ('queued','running')"
    )
    .fetch_all(&mut *conn)
    .await?;
    stop(conn, &ids).await
}
pub(crate) async fn stop(conn: &mut SqliteConnection, ids: &[i64]) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"DELETE
        FROM crawl_page
        WHERE work_state != 'done' AND candidate_id IN (SELECT value FROM json_each(?))"#,
        Json(ids)
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query!(
        r#"UPDATE candidate
        SET enrich = 'none'
        WHERE id IN (SELECT value FROM json_each(?)) AND enrich IN ('queued',
            'running')"#,
        Json(ids)
    )
    .execute(conn)
    .await?;
    Ok(())
}
