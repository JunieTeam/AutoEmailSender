//! Local statistics, derived from contact events and the current mentor and model data.

mod period;
pub use period::{Range, Window, first_event};
mod today;
pub use today::{TodayDrafts, TodayWork, today_work};

use failure::Failure;
use model::pricing::{Currency, Prices, Pricing};
use model::usage::{Purpose, Usage};
use serde::{Deserialize, Serialize};
use sqlx::{SqliteConnection, SqlitePool, types::Json};
use ts_rs::TS;

#[derive(Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Trend {
    pub date: String,
    pub sent: i64,
    pub replies: i64,
}

#[derive(Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SchoolProgress {
    pub university: String,
    pub contacted: i64,
    pub replied: i64,
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ContactNumbers {
    pub contacted: i64,
    pub replied: i64,
    pub median_days: Option<f64>,
    pub bounced: i64,
    pub waiting: [i64; 5],
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SchoolProgressPage {
    pub schools: Vec<SchoolProgress>,
    pub school_count: i64,
}

#[derive(Debug, Serialize, TS)]
pub struct ContactProgress {
    #[serde(flatten)]
    #[ts(flatten)]
    pub numbers: ContactNumbers,
    #[serde(flatten)]
    #[ts(flatten)]
    pub school_page: SchoolProgressPage,
    pub trend: Vec<Trend>,
}

pub const SCHOOL_PAGE_SIZE: i64 = 10;

/// The caller holds a read transaction, including the all-time series' first event.
pub async fn contact(
    conn: &mut SqliteConnection,
    window: &Window,
    page: std::num::NonZeroU32,
) -> Result<ContactProgress, sqlx::Error> {
    let numbers = contact_numbers(conn, window).await?;
    let school_page = contact_schools(conn, window, page).await?;
    let trend = contact_trend(conn, window).await?;
    Ok(ContactProgress {
        numbers,
        school_page,
        trend,
    })
}

pub async fn contact_numbers(
    conn: &mut SqliteConnection,
    window: &Window,
) -> Result<ContactNumbers, sqlx::Error> {
    let start = window.start;
    let end = window.end;
    let row = sqlx::query_file!("src/contact.sql", start, end)
        .fetch_one(conn)
        .await?;
    Ok(ContactNumbers {
        contacted: row.contacted,
        replied: row.replied,
        median_days: row.median_days,
        bounced: row.bounced,
        waiting: decode(&row.waiting)?,
    })
}

pub async fn contact_schools(
    conn: &mut SqliteConnection,
    window: &Window,
    page: std::num::NonZeroU32,
) -> Result<SchoolProgressPage, sqlx::Error> {
    let start = window.start;
    let end = window.end;
    let offset = i64::from(page.get() - 1) * SCHOOL_PAGE_SIZE;
    let row = sqlx::query_file!(
        "src/contact-schools.sql",
        start,
        end,
        SCHOOL_PAGE_SIZE,
        offset
    )
    .fetch_one(conn)
    .await?;
    Ok(SchoolProgressPage {
        schools: decode(&row.schools)?,
        school_count: row.school_count,
    })
}

pub async fn contact_trend(
    conn: &mut SqliteConnection,
    window: &Window,
) -> Result<Vec<Trend>, sqlx::Error> {
    let buckets = Json(&window.buckets);
    sqlx::query_file_as!(Trend, "src/trend.sql", buckets)
        .fetch_all(conn)
        .await
}

fn decode<T: serde::de::DeserializeOwned>(json: &str) -> Result<T, sqlx::Error> {
    serde_json::from_str(json).map_err(|e| sqlx::Error::Decode(Box::new(e)))
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct SchoolCount {
    pub university: String,
    pub count: i64,
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MatchingDistribution {
    pub total: i64,
    pub matching: [i64; 6],
}

#[derive(Debug, Serialize, TS)]
pub struct Completeness {
    pub email: i64,
    pub research: i64,
    pub papers: i64,
    pub homepage: i64,
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SchoolDistribution {
    pub schools: Vec<SchoolCount>,
    pub other_schools: i64,
}

#[derive(Debug, Serialize, TS)]
pub struct MentorOverview {
    #[serde(flatten)]
    #[ts(flatten)]
    pub distribution: MatchingDistribution,
    #[serde(flatten)]
    #[ts(flatten)]
    pub completeness: Completeness,
    #[serde(flatten)]
    #[ts(flatten)]
    pub school_distribution: SchoolDistribution,
}

pub async fn mentors(db: &SqlitePool) -> Result<MentorOverview, sqlx::Error> {
    let mut tx = db.begin().await?;
    let distribution = mentor_matching(&mut tx).await?;
    let completeness = mentor_completeness(&mut tx).await?;
    let school_distribution = mentor_schools(&mut tx).await?;
    tx.commit().await?;
    Ok(MentorOverview {
        distribution,
        completeness,
        school_distribution,
    })
}

pub async fn mentor_matching(
    conn: &mut SqliteConnection,
) -> Result<MatchingDistribution, sqlx::Error> {
    let row = sqlx::query_file!("src/matching.sql")
        .fetch_one(conn)
        .await?;
    Ok(MatchingDistribution {
        total: row.total,
        matching: decode(&row.matching)?,
    })
}

pub async fn mentor_completeness(conn: &mut SqliteConnection) -> Result<Completeness, sqlx::Error> {
    let row = sqlx::query_file!("src/completeness.sql")
        .fetch_one(conn)
        .await?;
    Ok(Completeness {
        email: row.email,
        research: row.research,
        papers: row.papers,
        homepage: row.homepage,
    })
}

pub async fn mentor_schools(
    conn: &mut SqliteConnection,
) -> Result<SchoolDistribution, sqlx::Error> {
    let row = sqlx::query_file!("src/mentor-schools.sql")
        .fetch_one(conn)
        .await?;
    Ok(SchoolDistribution {
        schools: decode(&row.schools)?,
        other_schools: row.other_schools,
    })
}

#[derive(Debug, Default, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TokenTotals {
    pub input: i64,
    pub cached: i64,
    pub output: i64,
    pub estimated_cost: Costs,
    pub unpriced_calls: i64,
}

/// Amounts in each currency, which are not converted; None where no call was priced in it.
#[derive(Debug, Default, Serialize, TS)]
pub struct Costs {
    pub cny: Option<f64>,
    pub usd: Option<f64>,
}

impl Costs {
    fn add(&mut self, (currency, amount): (Currency, f64)) {
        let sum = match currency {
            Currency::Cny => &mut self.cny,
            Currency::Usd => &mut self.usd,
        };
        *sum.get_or_insert(0.0) += amount;
    }
}

impl TokenTotals {
    fn add(&mut self, usage: Usage, cost: Option<(Currency, f64)>, calls: i64) {
        self.input += usage.input;
        self.cached += usage.cached;
        self.output += usage.output;
        match cost {
            Some(cost) => self.estimated_cost.add(cost),
            None => self.unpriced_calls += calls,
        }
    }
}

/// What `usage` costs at a configuration's prices now, and in the peak or off-peak hours it was
/// called in: none once the configuration is deleted, which takes its prices, or without them.
fn cost(
    currency: Option<Currency>,
    prices: Prices,
    off_peak_prices: Prices,
    off_peak: bool,
    usage: Usage,
) -> Option<(Currency, f64)> {
    let pricing = Pricing {
        currency: currency?,
        prices,
        off_peak: off_peak_prices,
    };
    Some((pricing.currency, pricing.cost(off_peak, usage)?))
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PurposeUsage {
    pub purpose: Purpose,
    #[serde(flatten)]
    #[ts(flatten)]
    pub tokens: TokenTotals,
}

#[derive(Debug, Serialize, TS)]
pub struct TokenUsage {
    pub purposes: Vec<PurposeUsage>,
    pub total: TokenTotals,
}

/// Costs are estimated with the configurations' prices now, and a call's peak or off-peak hours.
pub async fn tokens(db: &SqlitePool, window: &Window) -> Result<TokenUsage, sqlx::Error> {
    usage(db, window.start, window.end, None).await
}

/// What a crawl's calls took, and are estimated to cost as `tokens` estimates.
pub async fn crawl_tokens(db: &SqlitePool, crawl: i64) -> Result<TokenTotals, sqlx::Error> {
    Ok(usage(db, None, i64::MAX, Some(crawl)).await?.total)
}

async fn usage(
    db: &SqlitePool,
    start: Option<i64>,
    end: i64,
    crawl: Option<i64>,
) -> Result<TokenUsage, sqlx::Error> {
    let rows = sqlx::query_file!("src/tokens.sql", start, end, crawl)
        .fetch_all(db)
        .await?;
    let mut purposes: Vec<PurposeUsage> = Vec::new();
    let mut total = TokenTotals::default();
    for r in rows {
        let usage = Usage {
            input: r.input,
            cached: r.cached,
            output: r.output,
        };
        let cost = cost(
            r.currency,
            Prices {
                input: r.input_price,
                cache: r.cache_price,
                output: r.output_price,
            },
            Prices {
                input: r.off_peak_input_price,
                cache: r.off_peak_cache_price,
                output: r.off_peak_output_price,
            },
            r.off_peak,
            usage,
        );
        match purposes.last_mut() {
            Some(last) if last.purpose == r.purpose => last.tokens.add(usage, cost, r.calls),
            _ => {
                let mut tokens = TokenTotals::default();
                tokens.add(usage, cost, r.calls);
                purposes.push(PurposeUsage {
                    purpose: r.purpose,
                    tokens,
                });
            }
        }
        total.add(usage, cost, r.calls);
    }
    Ok(TokenUsage { purposes, total })
}

/// One call to a model (docs/ui.md 统计).
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ModelCall {
    pub id: i64,
    pub at: i64,
    pub purpose: Purpose,
    pub model: String,
    /// None for a call for no mentor, or once theirs is deleted.
    pub mentor: Option<CallMentor>,
    pub crawl_id: Option<i64>,
    pub result: CallResult,
}

#[derive(Debug, Serialize, TS)]
pub struct CallMentor {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CallResult {
    /// With what it took if the provider said, and its cost estimated as `tokens` estimates it.
    Answered {
        tokens: Option<Usage>,
        cost: Costs,
    },
    Failed {
        failure: Failure,
    },
}

#[derive(Debug, Serialize, TS)]
pub struct ModelCallPage {
    pub calls: Vec<ModelCall>,
    pub total: i64,
    /// The models called in the window, to filter by.
    pub models: Vec<String>,
}

pub const CALLS_PER_PAGE: u32 = 50;

/// The calls in `window`, the newest first, for `purpose` and `model` where given.
pub async fn calls(
    db: &SqlitePool,
    window: &Window,
    purpose: Option<Purpose>,
    model: Option<&str>,
    page: std::num::NonZeroU32,
) -> Result<ModelCallPage, sqlx::Error> {
    let limit = i64::from(CALLS_PER_PAGE);
    let offset = i64::from(page.get() - 1) * limit;
    let mut tx = db.begin().await?;
    let rows = sqlx::query_file!(
        "src/calls.sql",
        window.start,
        window.end,
        purpose,
        model,
        limit,
        offset
    )
    .fetch_all(&mut *tx)
    .await?;
    let total = sqlx::query_scalar!(
        r#"SELECT count(*) AS "total!: i64" FROM usage
        WHERE (?1 IS NULL OR at>=?1) AND at<?2 AND (?3 IS NULL OR purpose=?3)
          AND (?4 IS NULL OR model=?4)"#,
        window.start,
        window.end,
        purpose,
        model
    )
    .fetch_one(&mut *tx)
    .await?;
    let models = sqlx::query_scalar!(
        "SELECT DISTINCT model FROM usage WHERE (?1 IS NULL OR at>=?1) AND at<?2 ORDER BY model",
        window.start,
        window.end
    )
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let calls = rows
        .into_iter()
        .map(|r| {
            let result = match r.error {
                Some(Json(failure)) => CallResult::Failed { failure },
                None => {
                    // The table keeps the three counts together.
                    let tokens = match (r.input_tokens, r.cached_tokens, r.output_tokens) {
                        (Some(input), Some(cached), Some(output)) => Some(Usage {
                            input,
                            cached,
                            output,
                        }),
                        _ => None,
                    };
                    let mut estimated = Costs::default();
                    if let Some(usage) = tokens
                        && let Some(c) = cost(
                            r.currency,
                            Prices {
                                input: r.input_price,
                                cache: r.cache_price,
                                output: r.output_price,
                            },
                            Prices {
                                input: r.off_peak_input_price,
                                cache: r.off_peak_cache_price,
                                output: r.off_peak_output_price,
                            },
                            r.off_peak,
                            usage,
                        )
                    {
                        estimated.add(c);
                    }
                    CallResult::Answered {
                        tokens,
                        cost: estimated,
                    }
                }
            };
            ModelCall {
                id: r.id,
                at: r.at,
                purpose: r.purpose,
                model: r.model,
                mentor: r
                    .mentor_id
                    .zip(r.mentor_name)
                    .map(|(id, name)| CallMentor { id, name }),
                crawl_id: r.crawl_id,
                result,
            }
        })
        .collect();
    Ok(ModelCallPage {
        calls,
        total,
        models,
    })
}
