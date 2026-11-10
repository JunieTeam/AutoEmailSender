//! Model prices, and what calls are charged by them (docs/design.md#联系, 统计).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::providers::PeakHours;
use crate::usage::Usage;

/// What prices are in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, TS)]
#[serde(rename_all = "lowercase")]
#[sqlx(rename_all = "lowercase")]
pub enum Currency {
    /// Yuan.
    Cny,
    Usd,
}

/// Prices per million tokens, each of which may be left out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(rename = "ModelPrices")]
pub struct Prices {
    pub input: Option<f64>,
    /// Of the input tokens read from the provider's cache, which are otherwise charged as input.
    pub cache: Option<f64>,
    pub output: Option<f64>,
}

impl Prices {
    /// A provider's prices, of which only the cache's may be unpublished.
    pub const fn official(input: f64, cache: Option<f64>, output: f64) -> Prices {
        Prices {
            input: Some(input),
            cache,
            output: Some(output),
        }
    }
}

/// A configuration's prices.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Pricing {
    pub currency: Currency,
    /// At peak hours, where the provider has them.
    pub prices: Prices,
    /// Only where the provider has peak hours.
    pub off_peak: Prices,
}

/// Which of the prices of a provider with peak hours a call is charged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Hours {
    Peak,
    OffPeak,
}

/// What a call is charged per million tokens, for estimates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "ModelRate")]
pub struct Rate {
    pub input: f64,
    pub output: f64,
    pub currency: Currency,
    /// Only where the provider has peak hours.
    pub hours: Option<Hours>,
}

/// Whether a call at `at`, in UTC milliseconds, falls in a provider's off-peak hours.
pub fn off_peak(peak_hours: Option<&PeakHours>, at: i64) -> bool {
    peak_hours.is_some_and(|h| !h.contain(at))
}

impl Pricing {
    /// The prices a call is charged, and whether they are the off-peak ones: those need their
    /// input and output prices filled in, or the others stand.
    fn charged(&self, off_peak: bool) -> (Prices, bool) {
        if off_peak && self.off_peak.input.is_some() && self.off_peak.output.is_some() {
            (self.off_peak, true)
        } else {
            (self.prices, false)
        }
    }

    /// What a call cost; None unless its input and output prices are filled in.
    pub fn cost(&self, off_peak: bool, usage: Usage) -> Option<f64> {
        let (prices, _) = self.charged(off_peak);
        let (input, output) = (prices.input?, prices.output?);
        let cache = prices.cache.unwrap_or(input);
        let tokens = |n: i64| n as f64;
        Some(
            (tokens(usage.input - usage.cached) * input
                + tokens(usage.cached) * cache
                + tokens(usage.output) * output)
                / 1_000_000.0,
        )
    }

    /// What a call at `at` is charged, for a provider with these peak hours; None unless its input
    /// and output prices are filled in.
    pub fn rate(&self, peak_hours: Option<&PeakHours>, at: i64) -> Option<Rate> {
        let (prices, off) = self.charged(off_peak(peak_hours, at));
        Some(Rate {
            input: prices.input?,
            output: prices.output?,
            currency: self.currency,
            hours: peak_hours.map(|_| if off { Hours::OffPeak } else { Hours::Peak }),
        })
    }
}
