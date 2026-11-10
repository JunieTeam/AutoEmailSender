//! The provider table: what differs between model providers lives here, as data
//! (docs/design.md#桌面端). It follows each provider's official documentation, checked again before
//! each release (docs/development.md#发布).

use serde::Serialize;
use serde_json::{Value, json};
use ts_rs::TS;

use crate::pricing::{Currency, Prices};

/// The provider id of a model configuration with an address the user filled in.
pub const CUSTOM: &str = "custom";

// Named apart from the mail providers for the UI.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "ModelProvider")]
pub struct Provider {
    pub id: &'static str,
    pub name: &'static str,
    /// Where it is called, the first by default: one for each region where its regions have API
    /// keys of their own, else just one.
    pub endpoints: &'static [Endpoint],
    pub currency: Currency,
    /// None where it charges the same at all hours.
    pub peak_hours: Option<PeakHours>,
    /// Its main text models, with the prices of its first endpoint.
    pub models: &'static [KnownModel],
    /// How its other models are told not to think, where one way works for all of them. Most
    /// providers also serve models that cannot stop thinking, such as GLM-5.3 and Kimi K3.
    #[serde(skip)]
    #[ts(skip)]
    pub thinking: Option<Thinking>,
    /// All its models accept `response_format: {"type":"json_object"}`.
    #[serde(skip)]
    #[ts(skip)]
    pub json_mode: bool,
    /// Its model list gives each model's prices in dollars per token, the way OpenRouter's does.
    #[serde(skip)]
    #[ts(skip)]
    pub priced_list: bool,
    /// Answers 429 with OpenAI's words for a used-up balance ("insufficient_quota", "You exceeded
    /// your current quota") when only a per-minute token limit is hit; its balance errors say
    /// otherwise.
    #[serde(skip)]
    #[ts(skip)]
    pub quota_is_rate_limit: bool,
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "ModelEndpoint")]
pub struct Endpoint {
    /// None for a provider's only endpoint.
    pub region: Option<Region>,
    pub base_url: &'static str,
    /// Where the user gets an API key.
    pub key_url: &'static str,
}

#[derive(Debug, Clone, Copy, Serialize, TS)]
#[ts(rename = "ModelRegion")]
pub struct Region {
    /// The provider's own id for it, which configurations store.
    pub id: &'static str,
    pub name: &'static str,
}

/// When a provider charges its peak prices; at other times it charges its off-peak ones.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PeakHours {
    /// The time the hours are in, as the UI says it.
    pub zone: &'static str,
    /// How many hours that time is ahead of UTC.
    #[serde(skip)]
    #[ts(skip)]
    pub utc_offset: i64,
    /// From and until which hour, Monday to Friday.
    pub weekdays: &'static [(u8, u8)],
}

impl PeakHours {
    /// Whether `at`, in UTC milliseconds, is in them.
    pub fn contain(&self, at: i64) -> bool {
        let hours = at.div_euclid(3_600_000) + self.utc_offset;
        // 1970-01-01 was a Thursday, three days after a Monday.
        let weekday = (hours.div_euclid(24) + 3).rem_euclid(7);
        let hour = hours.rem_euclid(24);
        weekday < 5
            && self
                .weekdays
                .iter()
                .any(|&(from, until)| (i64::from(from)..i64::from(until)).contains(&hour))
    }
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct KnownModel {
    pub name: &'static str,
    /// Its official prices: at peak hours where the provider has them, the lowest band where they
    /// rise with the input's length, and no limited-time discounts.
    pub prices: Prices,
    /// Only where the provider has peak hours.
    pub off_peak: Option<Prices>,
    #[serde(skip)]
    #[ts(skip)]
    pub thinking: Option<Thinking>,
    #[serde(skip)]
    #[ts(skip)]
    pub json_mode: bool,
}

/// How a model is told to think as little as it can. Not sending it, many think as hard as they
/// may, slow and costly.
#[derive(Debug, Clone, Copy)]
pub enum Thinking {
    /// `"thinking": {"type": "disabled"}`
    Disabled,
    /// `"enable_thinking": false`
    EnableThinkingFalse,
    /// `"reasoning_effort"` at its lowest, for a model that cannot stop thinking or stops at
    /// `"none"`.
    Effort(&'static str),
}

impl Thinking {
    /// Adds the parameter to a chat request.
    pub(crate) fn apply(self, body: &mut Value) {
        match self {
            Thinking::Disabled => body["thinking"] = json!({"type": "disabled"}),
            Thinking::EnableThinkingFalse => body["enable_thinking"] = false.into(),
            Thinking::Effort(effort) => body["reasoning_effort"] = effort.into(),
        }
    }
}

impl Provider {
    /// Its endpoint in the region; the first where none is given or it has no regions.
    pub fn endpoint(&self, region: Option<&str>) -> Option<&'static Endpoint> {
        match region {
            Some(id) if self.endpoints.len() > 1 => self
                .endpoints
                .iter()
                .find(|e| e.region.is_some_and(|r| r.id == id)),
            _ => self.endpoints.first(),
        }
    }

    /// How to ask the model not to think and whether it takes JSON mode: as its documentation
    /// says if it is known, else as all this provider's models take.
    pub(crate) fn asking(&self, model: &str) -> (Option<Thinking>, bool) {
        match self.models.iter().find(|m| m.name == model) {
            Some(m) => (m.thinking, m.json_mode),
            None => (self.thinking, self.json_mode),
        }
    }
}

pub fn find(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.id == id)
}

pub static PROVIDERS: &[Provider] = &[
    Provider {
        id: "deepseek",
        name: "DeepSeek",
        endpoints: &[Endpoint {
            region: None,
            base_url: "https://api.deepseek.com",
            key_url: "https://platform.deepseek.com/api_keys",
        }],
        currency: Currency::Cny,
        // It also counts public holidays as off-peak, which are not known here.
        peak_hours: Some(PeakHours {
            zone: "北京时间",
            utc_offset: 8,
            weekdays: &[(9, 12), (14, 18)],
        }),
        models: &[
            KnownModel {
                name: "deepseek-flash",
                prices: Prices::official(2.0, Some(0.04), 8.0),
                off_peak: Some(Prices::official(1.0, Some(0.02), 4.0)),
                thinking: Some(Thinking::Disabled),
                json_mode: true,
            },
            KnownModel {
                name: "deepseek-v4-pro",
                prices: Prices::official(9.0, Some(0.3), 27.0),
                off_peak: Some(Prices::official(4.5, Some(0.15), 13.5)),
                thinking: Some(Thinking::Disabled),
                json_mode: true,
            },
        ],
        thinking: Some(Thinking::Disabled),
        // The JSON Output guide documents json_object without model restrictions.
        json_mode: true,
        priced_list: false,
        quota_is_rate_limit: false,
    },
    Provider {
        id: "qwen",
        name: "通义千问（阿里云百炼）",
        // Frankfurt and Tokyo are served only on a workspace's own domain.
        endpoints: &[
            Endpoint {
                region: Some(Region {
                    id: "cn-beijing",
                    name: "北京",
                }),
                base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
                key_url: "https://bailian.console.aliyun.com/cn-beijing/model/settings/api-key",
            },
            Endpoint {
                region: Some(Region {
                    id: "ap-southeast-1",
                    name: "新加坡",
                }),
                base_url: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
                key_url: "https://bailian.console.aliyun.com/ap-southeast-1/model/settings/api-key",
            },
            Endpoint {
                region: Some(Region {
                    id: "us-east-1",
                    name: "美国（弗吉尼亚）",
                }),
                base_url: "https://dashscope-us.aliyuncs.com/compatible-mode/v1",
                key_url: "https://bailian.console.aliyun.com/us-east-1/model/settings/api-key",
            },
            Endpoint {
                region: Some(Region {
                    id: "cn-hongkong",
                    name: "中国香港",
                }),
                base_url: "https://cn-hongkong.dashscope.aliyuncs.com/compatible-mode/v1",
                key_url: "https://bailian.console.aliyun.com/cn-hongkong/model/settings/api-key",
            },
        ],
        currency: Currency::Cny,
        peak_hours: None,
        // Implicit cache hits cost a fifth of the input price, except on the models whose price
        // the context cache guide leaves to the console.
        models: &[
            KnownModel {
                name: "qwen3.8-max",
                prices: Prices::official(12.0, None, 36.0),
                off_peak: None,
                thinking: Some(Thinking::EnableThinkingFalse),
                json_mode: true,
            },
            KnownModel {
                name: "qwen3.7-plus",
                prices: Prices::official(2.0, Some(0.4), 8.0),
                off_peak: None,
                thinking: Some(Thinking::EnableThinkingFalse),
                json_mode: true,
            },
            KnownModel {
                name: "qwen3.8-flash",
                prices: Prices::official(0.8, None, 2.7),
                off_peak: None,
                thinking: Some(Thinking::EnableThinkingFalse),
                json_mode: true,
            },
        ],
        thinking: None,
        // The JSON Mode guide limits support by model and thinking mode.
        json_mode: false,
        priced_list: false,
        // The error code guide lists them under Throttling.AllocationQuota, a TPM limit; a
        // balance owed comes as Arrearage.
        quota_is_rate_limit: true,
    },
    Provider {
        id: "glm",
        name: "智谱 GLM",
        endpoints: &[Endpoint {
            region: None,
            base_url: "https://open.bigmodel.cn/api/paas/v4",
            key_url: "https://bigmodel.cn/usercenter/proj-mgmt/apikeys",
        }],
        currency: Currency::Cny,
        peak_hours: None,
        models: &[
            KnownModel {
                name: "glm-5.3",
                prices: Prices::official(8.0, Some(2.0), 28.0),
                off_peak: None,
                thinking: Some(Thinking::Effort("low")),
                json_mode: true,
            },
            KnownModel {
                name: "glm-5.3-flash",
                prices: Prices::official(0.8, Some(0.23), 2.8),
                off_peak: None,
                thinking: Some(Thinking::Effort("low")),
                // Its documentation does not say it takes JSON mode.
                json_mode: false,
            },
            KnownModel {
                name: "glm-4.7-flash",
                prices: Prices::official(0.0, Some(0.0), 0.0),
                off_peak: None,
                thinking: Some(Thinking::Disabled),
                json_mode: true,
            },
        ],
        thinking: None,
        // The Chat API reference restricts response_format to text models.
        json_mode: false,
        priced_list: false,
        quota_is_rate_limit: false,
    },
    Provider {
        id: "doubao",
        name: "豆包（火山方舟）",
        endpoints: &[Endpoint {
            region: None,
            base_url: "https://ark.cn-beijing.volces.com/api/v3",
            key_url: "https://console.volcengine.com/ark/region:ark+cn-beijing/apiKey",
        }],
        currency: Currency::Cny,
        peak_hours: None,
        models: &[
            KnownModel {
                name: "doubao-seed-2-1-pro-260915",
                prices: Prices::official(6.0, Some(1.2), 30.0),
                off_peak: None,
                thinking: Some(Thinking::Disabled),
                json_mode: true,
            },
            KnownModel {
                name: "doubao-seed-2-1-lite-260915",
                prices: Prices::official(0.8, Some(0.16), 2.7),
                off_peak: None,
                thinking: Some(Thinking::Disabled),
                json_mode: true,
            },
        ],
        thinking: None,
        // The structured-output guide limits JSON mode to listed models and inference options.
        json_mode: false,
        priced_list: false,
        quota_is_rate_limit: false,
    },
    Provider {
        id: "kimi",
        name: "Kimi（月之暗面）",
        endpoints: &[Endpoint {
            region: None,
            base_url: "https://api.moonshot.cn/v1",
            key_url: "https://platform.moonshot.cn/console/api-keys",
        }],
        currency: Currency::Cny,
        peak_hours: None,
        models: &[
            KnownModel {
                name: "kimi-k3.1-pro",
                prices: Prices::official(13.0, Some(1.3), 68.0),
                off_peak: None,
                thinking: Some(Thinking::Effort("low")),
                json_mode: true,
            },
            KnownModel {
                name: "kimi-k3",
                prices: Prices::official(20.0, Some(2.0), 100.0),
                off_peak: None,
                thinking: Some(Thinking::Effort("low")),
                json_mode: true,
            },
        ],
        thinking: None,
        // The JSON Mode guide documents json_object across Kimi models, including K3.1 Pro.
        json_mode: true,
        priced_list: false,
        quota_is_rate_limit: false,
    },
    Provider {
        id: "siliconflow",
        name: "硅基流动",
        endpoints: &[Endpoint {
            region: None,
            base_url: "https://api.siliconflow.cn/v1",
            key_url: "https://cloud.siliconflow.cn/account/ak",
        }],
        currency: Currency::Cny,
        peak_hours: None,
        // Its reasoning guide does not say which models take `enable_thinking`.
        models: &[
            KnownModel {
                name: "deepseek-ai/DeepSeek-V4-Pro",
                prices: Prices::official(12.0, Some(1.0), 24.0),
                off_peak: None,
                thinking: None,
                json_mode: true,
            },
            KnownModel {
                name: "zai-org/GLM-5.3",
                prices: Prices::official(8.0, Some(2.0), 28.0),
                off_peak: None,
                thinking: None,
                json_mode: true,
            },
            KnownModel {
                name: "Qwen/Qwen3.8-27B",
                prices: Prices::official(3.0, None, 12.0),
                off_peak: None,
                thinking: None,
                json_mode: true,
            },
        ],
        thinking: None,
        // The JSON Mode guide excludes VL models, so even json_object is not universal.
        json_mode: false,
        priced_list: false,
        quota_is_rate_limit: false,
    },
    Provider {
        id: "openai",
        name: "OpenAI",
        endpoints: &[Endpoint {
            region: None,
            base_url: "https://api.openai.com/v1",
            key_url: "https://platform.openai.com/api-keys",
        }],
        currency: Currency::Usd,
        peak_hours: None,
        models: &[
            KnownModel {
                name: "gpt-6.1-sol",
                prices: Prices::official(2.0, Some(0.1), 10.0),
                off_peak: None,
                thinking: Some(Thinking::Effort("low")),
                json_mode: true,
            },
            KnownModel {
                name: "gpt-6.1-terra",
                prices: Prices::official(1.0, Some(0.05), 5.0),
                off_peak: None,
                thinking: Some(Thinking::Effort("none")),
                json_mode: true,
            },
            KnownModel {
                name: "gpt-6-luna",
                prices: Prices::official(0.1, Some(0.01), 0.5),
                off_peak: None,
                thinking: Some(Thinking::Effort("none")),
                json_mode: true,
            },
        ],
        thinking: None,
        // The guide's JSON Mode compatibility table does not cover all Chat models.
        json_mode: false,
        priced_list: false,
        quota_is_rate_limit: false,
    },
    Provider {
        id: "gemini",
        name: "Google Gemini",
        endpoints: &[Endpoint {
            region: None,
            base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
            key_url: "https://aistudio.google.com/apikey",
        }],
        currency: Currency::Usd,
        peak_hours: None,
        // Its OpenAI compatibility guide offers structured output only as a JSON schema.
        models: &[
            KnownModel {
                name: "gemini-3.8-flash",
                prices: Prices::official(1.5, Some(0.15), 7.5),
                off_peak: None,
                thinking: Some(Thinking::Effort("minimal")),
                json_mode: false,
            },
            KnownModel {
                name: "gemini-3.5-flash-lite",
                prices: Prices::official(0.3, Some(0.03), 2.5),
                off_peak: None,
                thinking: Some(Thinking::Effort("minimal")),
                json_mode: false,
            },
        ],
        thinking: None,
        // Image model cards exclude structured output; JSON mode has no all-model guarantee.
        json_mode: false,
        priced_list: false,
        quota_is_rate_limit: false,
    },
    Provider {
        id: "openrouter",
        name: "OpenRouter",
        endpoints: &[Endpoint {
            region: None,
            base_url: "https://openrouter.ai/api/v1",
            key_url: "https://openrouter.ai/keys",
        }],
        currency: Currency::Usd,
        peak_hours: None,
        // It serves hundreds, whose prices come with its model list.
        models: &[],
        thinking: None,
        // JSON mode has no guarantee across all models and provider endpoints.
        json_mode: false,
        priced_list: true,
        quota_is_rate_limit: false,
    },
];
