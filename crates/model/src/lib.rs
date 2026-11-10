//! Models: the provider table, the user's model configurations, and calling OpenAI-compatible
//! APIs (docs/design.md#桌面端).

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use ts_rs::TS;

pub mod background;
mod client;
mod error;
pub mod output;
pub mod pricing;
pub mod providers;
pub mod usage;

pub use client::{Access, Answer, CUT_OFF, Finish, ListedModel, ask, list_models, test};
pub use error::ModelError;
use pricing::{Currency, Prices, Pricing, Rate};

/// A model configuration as the settings show it. Its API key never leaves the core.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfig {
    pub id: i64,
    /// None: known by the model's name.
    pub name: Option<String>,
    /// A provider's id, or `providers::CUSTOM`.
    pub provider: String,
    /// Only for a provider with regions.
    pub region: Option<String>,
    /// Only for `providers::CUSTOM`.
    pub base_url: Option<String>,
    pub model: String,
    /// The one AI features use: the user's choice, or else the earliest added.
    pub is_default: bool,
    /// The user agreed to what AI features send to its provider.
    pub agreed: bool,
    pub needs_api_key: bool,
    #[serde(flatten)]
    #[ts(flatten)]
    pub pricing: Pricing,
    /// What a call made when it was listed is charged; None unless those prices are filled in.
    pub rate: Option<Rate>,
}

/// A model configuration as the user entered it, without its API key; `check` validates it.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ModelForm {
    /// May be blank.
    pub name: String,
    pub provider: String,
    /// Only read for a provider with regions, whose first it defaults to.
    pub region: Option<String>,
    /// Only read for `providers::CUSTOM`.
    pub base_url: String,
    pub model: String,
    /// None: the provider's.
    pub currency: Option<Currency>,
    /// Left out: none filled in.
    #[serde(default)]
    #[ts(as = "Option<_>", optional)]
    pub prices: Prices,
    /// Only for a provider with peak hours.
    #[serde(default)]
    #[ts(as = "Option<_>", optional)]
    pub off_peak: Prices,
}

/// A checked `ModelForm`.
#[derive(Debug)]
pub struct ModelFields {
    pub name: Option<String>,
    pub source: Source,
    pub model: String,
    pub pricing: Pricing,
}

/// Where a configuration calls.
#[derive(Debug)]
pub enum Source {
    Known(&'static providers::Provider, &'static providers::Endpoint),
    /// An OpenAI-compatible address the user filled in, without a trailing slash.
    Custom(String),
}

impl Source {
    /// From the provider, region and address as entered; the error says what to fix.
    pub fn check(provider: &str, region: Option<&str>, base_url: &str) -> Result<Source, String> {
        if provider != providers::CUSTOM {
            let provider = providers::find(provider).ok_or("请选择服务商")?;
            let endpoint = provider.endpoint(region).ok_or("请选择地域")?;
            return Ok(Source::Known(provider, endpoint));
        }
        // Calls append `/chat/completions` or `/responses`, and `/models`. Relay services often
        // show the whole address of the API they offer, which is then pasted.
        let base_url = base_url.trim().trim_end_matches('/');
        let base_url = base_url
            .strip_suffix("/chat/completions")
            .or_else(|| base_url.strip_suffix("/responses"))
            .unwrap_or(base_url);
        let Some(url) = url::Url::parse(base_url)
            .ok()
            .filter(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some())
        else {
            return Err("接口地址不正确，需以 http:// 或 https:// 开头".into());
        };
        // The address is no secret: it is listed, logged and backed up. A key goes in its own field.
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(
                "接口地址只填到路径，不带用户名、密码和 ? 后面的参数；API Key 填在 API Key 一栏"
                    .into(),
            );
        }
        Ok(Source::Custom(base_url.to_owned()))
    }

    fn id(&self) -> &'static str {
        match self {
            Source::Known(p, _) => p.id,
            Source::Custom(_) => providers::CUSTOM,
        }
    }

    /// The region to store: only that of a provider with regions.
    fn region(&self) -> Option<&'static str> {
        match self {
            Source::Known(_, e) => e.region.map(|r| r.id),
            Source::Custom(_) => None,
        }
    }

    /// The address to store: only a custom one.
    fn custom_url(&self) -> Option<&str> {
        match self {
            Source::Known(..) => None,
            Source::Custom(url) => Some(url),
        }
    }

    /// Where and how to call, with this API key.
    pub fn access(&self, api_key: Option<String>) -> Access {
        match self {
            Source::Known(p, e) => Access {
                base_url: e.base_url.into(),
                api_key,
                provider: Some(p),
            },
            Source::Custom(url) => Access {
                base_url: url.clone(),
                api_key,
                provider: None,
            },
        }
    }
}

impl ModelForm {
    /// Trims the text and checks every field; the error says what to fix.
    pub fn check(self) -> Result<ModelFields, String> {
        let source = Source::check(&self.provider, self.region.as_deref(), &self.base_url)?;
        let model = self.model.trim();
        if model.is_empty() {
            return Err("请填写模型名".into());
        }
        if [self.prices, self.off_peak]
            .iter()
            .flat_map(|p| [p.input, p.cache, p.output])
            .flatten()
            .any(|p| p < 0.0)
        {
            return Err("单价不能是负数".into());
        }
        let provider = match &source {
            Source::Known(p, _) => Some(*p),
            Source::Custom(_) => None,
        };
        if provider.is_none_or(|p| p.peak_hours.is_none()) && self.off_peak != Prices::default() {
            return Err("这家服务商不分时段计价，不用填空闲时段的单价".into());
        }
        Ok(ModelFields {
            name: Some(self.name.trim())
                .filter(|n| !n.is_empty())
                .map(str::to_owned),
            pricing: Pricing {
                currency: self
                    .currency
                    .unwrap_or(provider.map_or(Currency::Cny, |p| p.currency)),
                prices: self.prices,
                off_peak: self.off_peak,
            },
            source,
            model: model.to_owned(),
        })
    }
}

/// An API key as pasted (`net::pasted_key`); None when left blank. A line break inside it would
/// fail every request as if the network were down, and the provider would only call a key with
/// other characters wrong.
pub fn checked_key(api_key: &str) -> Result<Option<String>, &'static str> {
    let key = net::pasted_key(api_key);
    if !key.is_ascii() {
        return Err("API Key 里有中文或其他多余的字符。请重新复制，只粘贴 Key 本身。");
    }
    Ok((!key.is_empty()).then_some(key))
}

/// Every model configuration, oldest first, with what a call at `now` is charged.
pub async fn list(db: &SqlitePool, now: i64) -> Result<Vec<ModelConfig>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT id, name, provider, region, base_url, model, currency AS "currency: Currency",
             input_price, cache_price, output_price, off_peak_input_price, off_peak_cache_price,
             off_peak_output_price,
             id = coalesce((SELECT default_model_config_id FROM profile),
                           (SELECT min(id) FROM model_config)) AS "is_default!: bool",
             agreed AS "agreed: bool", api_key IS NULL AS "needs_api_key!: bool"
           FROM model_config ORDER BY id"#
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let pricing = Pricing {
                currency: r.currency,
                prices: Prices {
                    input: r.input_price,
                    cache: r.cache_price,
                    output: r.output_price,
                },
                off_peak: Prices {
                    input: r.off_peak_input_price,
                    cache: r.off_peak_cache_price,
                    output: r.off_peak_output_price,
                },
            };
            let peak_hours = providers::find(&r.provider).and_then(|p| p.peak_hours.as_ref());
            ModelConfig {
                id: r.id,
                name: r.name,
                provider: r.provider,
                region: r.region,
                base_url: r.base_url,
                model: r.model,
                is_default: r.is_default,
                agreed: r.agreed,
                needs_api_key: r.needs_api_key,
                rate: pricing.rate(peak_hours, now),
                pricing,
            }
        })
        .collect())
}

pub async fn create(
    db: &SqlitePool,
    fields: &ModelFields,
    api_key: &str,
    now: i64,
) -> Result<i64, sqlx::Error> {
    let s = &fields.source;
    let (provider, region, base_url) = (s.id(), s.region(), s.custom_url());
    let Pricing {
        currency,
        prices,
        off_peak,
    } = fields.pricing;
    let row = sqlx::query!(
        r#"INSERT INTO model_config (name, provider, region, base_url, model, api_key, agreed,
            created_at, currency, input_price, cache_price, output_price, off_peak_input_price,
            off_peak_cache_price, off_peak_output_price)
        VALUES (?, ?, ?, ?, ?, ?, 0, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        fields.name,
        provider,
        region,
        base_url,
        fields.model,
        api_key,
        now,
        currency,
        prices.input,
        prices.cache,
        prices.output,
        off_peak.input,
        off_peak.cache,
        off_peak.output,
    )
    .execute(db)
    .await?;
    Ok(row.last_insert_rowid())
}

/// False when there is no such configuration. What the user agreed to stays agreed only while the
/// provider, region and address stay the same.
pub async fn update(
    db: &SqlitePool,
    id: i64,
    fields: &ModelFields,
    api_key: &str,
) -> Result<bool, sqlx::Error> {
    let s = &fields.source;
    let (provider, region, base_url) = (s.id(), s.region(), s.custom_url());
    let Pricing {
        currency,
        prices,
        off_peak,
    } = fields.pricing;
    let row = sqlx::query!(
        r#"UPDATE model_config
        SET agreed = agreed AND provider = ?2 AND region IS ?3 AND base_url IS ?4,
            provider = ?2, region = ?3, base_url = ?4, name = ?5, model = ?6, api_key = ?7,
            currency = ?8, input_price = ?9, cache_price = ?10, output_price = ?11,
            off_peak_input_price = ?12, off_peak_cache_price = ?13, off_peak_output_price = ?14
        WHERE id = ?1"#,
        id,
        provider,
        region,
        base_url,
        fields.name,
        fields.model,
        api_key,
        currency,
        prices.input,
        prices.cache,
        prices.output,
        off_peak.input,
        off_peak.cache,
        off_peak.output,
    )
    .execute(db)
    .await?;
    Ok(row.rows_affected() > 0)
}

/// Records that the user agreed to what AI features send to this source, as they were told; false
/// when the configuration is gone or now calls somewhere else.
pub async fn agree(db: &SqlitePool, id: i64, source: &Source) -> Result<bool, sqlx::Error> {
    let (provider, region, base_url) = (source.id(), source.region(), source.custom_url());
    let row = sqlx::query!(
        "UPDATE model_config SET agreed = 1
        WHERE id = ? AND provider = ? AND region IS ? AND base_url IS ?",
        id,
        provider,
        region,
        base_url,
    )
    .execute(db)
    .await?;
    Ok(row.rows_affected() > 0)
}

/// The configuration AI features call, with its API key.
#[derive(Debug, Clone)]
pub struct Chosen {
    pub id: i64,
    pub provider: String,
    pub model: String,
    pub agreed: bool,
    pub access: Access,
}

/// The default configuration, as `list` marks it; None when there is none.
pub async fn chosen<'a>(
    db: impl sqlx::Executor<'a, Database = sqlx::Sqlite>,
) -> Result<Option<Chosen>, sqlx::Error> {
    selected(db, None).await
}

/// A task's configuration, falling back to the default only when it has been deleted.
pub async fn selected<'a>(
    db: impl sqlx::Executor<'a, Database = sqlx::Sqlite>,
    id: Option<i64>,
) -> Result<Option<Chosen>, sqlx::Error> {
    let Some(row) = sqlx::query!(
        r#"SELECT id, provider, region, base_url, model, api_key, agreed AS "agreed: bool"
        FROM model_config
        WHERE id = coalesce((SELECT id FROM model_config WHERE id = ?1),
            (SELECT default_model_config_id FROM profile),
            (SELECT min(id) FROM model_config))"#,
        id
    )
    .fetch_optional(db)
    .await?
    else {
        return Ok(None);
    };
    // Only a provider or region this version no longer knows fails.
    let source = Source::check(
        &row.provider,
        row.region.as_deref(),
        row.base_url.as_deref().unwrap_or_default(),
    )
    .map_err(|e| sqlx::Error::Decode(e.into()))?;
    Ok(Some(Chosen {
        id: row.id,
        provider: row.provider,
        model: row.model,
        agreed: row.agreed,
        access: source.access(row.api_key),
    }))
}

/// Deletes a configuration; what used it keeps the model's name (docs/data-model.md#删除).
pub async fn delete(db: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query!("DELETE FROM model_config WHERE id = ?", id)
        .execute(db)
        .await?;
    Ok(())
}

/// The saved API key of a configuration, for an edit that leaves it blank; None when the
/// configuration was saved for another provider, region or address, which must not get this key.
/// Some(None) is a matching restored configuration whose key must be filled in again.
pub async fn api_key(
    db: &SqlitePool,
    id: i64,
    source: &Source,
) -> Result<Option<Option<String>>, sqlx::Error> {
    let (provider, region, base_url) = (source.id(), source.region(), source.custom_url());
    sqlx::query_scalar!(
        "SELECT api_key FROM model_config
        WHERE id = ? AND provider = ? AND region IS ? AND base_url IS ?",
        id,
        provider,
        region,
        base_url,
    )
    .fetch_optional(db)
    .await
}
