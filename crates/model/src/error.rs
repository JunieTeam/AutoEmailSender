use std::error::Error;

use failure::Failure;

/// Why calling a model provider failed. Displays in English for the log, without the provider's
/// words, which may quote what was sent; `failure` is for the user and `diagnostic` for the log.
#[derive(Debug)]
pub enum ModelError {
    MissingKey,
    /// No answer at all: the network, the proxy, the address or its certificate.
    Unreachable {
        address: String,
        source: reqwest::Error,
    },
    TimedOut {
        address: String,
    },
    /// The provider answered with an error status, with its words.
    Refused {
        status: u16,
        reply: String,
        retry_after: Option<std::time::Duration>,
        code: Option<String>,
    },
    /// The provider failed partway through its reply, in its words.
    Failed(String),
    /// The model replied without an answer, such as when it spent everything on thinking.
    NoAnswer {
        finish_reason: Option<crate::Finish>,
    },
    /// The provider answered in a way that makes no sense.
    Broken(String),
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelError::MissingKey => write!(f, "API key missing after restore"),
            ModelError::Unreachable { address, source } => {
                // reqwest's own words are only "error sending request"; the cause is deeper.
                write!(f, "reach {address}")?;
                let mut next: Option<&dyn Error> = Some(source);
                while let Some(e) = next {
                    write!(f, ": {e}")?;
                    next = e.source();
                }
                Ok(())
            }
            ModelError::TimedOut { address } => write!(f, "{address} timed out"),
            // Its error code too is the provider's words, which may quote the key.
            ModelError::Refused { status, .. } => write!(f, "refused with {status}"),
            ModelError::Failed(_) => write!(f, "failed partway"),
            ModelError::NoAnswer { finish_reason } => {
                write!(f, "no answer, finish reason {finish_reason:?}")
            }
            ModelError::Broken(e) => write!(f, "{e}"),
        }
    }
}

impl Error for ModelError {}

impl ModelError {
    /// What happened and what to do about it; the provider's words only when no case fits them.
    pub fn failure(&self) -> Failure {
        match self {
            ModelError::MissingKey => {
                Failure::new("恢复后需要重新填写 API Key。请到“设置 › AI › 模型”填写后再试。")
            }
            ModelError::Unreachable { address, source } if net::untrusted(source) => Failure::new(
                format!("{address} 的证书不受信任，连接可能被拦截。请更换网络后重试。"),
            ),
            ModelError::Unreachable { address, .. } => Failure::new(format!(
                "无法连接 {address}。请检查网络是否连通；开启了系统代理的，请确认代理软件正在运行；\
                在国内使用 OpenAI、Gemini 这类国外服务需要开启代理。"
            )),
            ModelError::TimedOut { address } => Failure::new(format!(
                "{address} 长时间未响应。请稍后再试；在国内使用 OpenAI、Gemini 这类国外服务需要开启代理。"
            )),
            ModelError::Refused {
                status,
                reply,
                code,
                ..
            } => Cause::of(*status, code.as_deref(), reply).failure(reply),
            ModelError::Failed(reply) => match Cause::of(0, None, reply) {
                Cause::Other => Failure::unrecognized("服务商在回答途中出错。请稍后再试。", reply),
                cause => cause.failure(reply),
            },
            ModelError::NoAnswer { finish_reason } => Failure::new(format!(
                "模型未给出回答（结束原因：{}）。请更换模型后重试。",
                match finish_reason {
                    None => "未说明",
                    Some(crate::Finish::Stop) => "正常结束",
                    Some(crate::Finish::Length) => "达到长度上限",
                    Some(crate::Finish::ContentFilter) => "被服务商的内容审核拦下",
                    Some(crate::Finish::ToolCalls) => "要调用工具",
                    Some(crate::Finish::Other) => "服务商自己的原因",
                }
            )),
            ModelError::Broken(e) => Failure::unrecognized(
                "服务商的回复无法读取。请检查接口地址是否为 OpenAI 兼容接口。",
                e,
            ),
        }
    }

    /// The provider's words, for the log.
    pub fn diagnostic(&self) -> Option<String> {
        match self {
            ModelError::Refused { reply, code, .. } => Some(failure::diagnostic(&match code {
                Some(code) => format!("{code}: {reply}"),
                None => reply.clone(),
            })),
            ModelError::Failed(reply) => Some(failure::diagnostic(reply)),
            _ => None,
        }
    }
}

pub(crate) const TOO_LONG: &str =
    "资料超出了模型能读取的长度。减少参考材料，或换一个支持更长上下文的模型后重试。";
pub(crate) const NO_BALANCE: &str =
    "模型服务商的账户余额或额度不足。到服务商网站充值或补充额度后再试。";
pub(crate) const NO_MODEL: &str =
    "服务商找不到这个模型。请在“设置 › AI › 模型”里检查模型名或更换模型后再试。";
pub(crate) const NOT_ORDERED: &str =
    "服务商尚未开通这项服务，或用量已达到你在服务商处设置的上限。到服务商网站查看后再试。";

/// What a provider's refusal is about, by its status, error code and words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cause {
    TooLong,
    NoBalance,
    /// "Quota exceeded": the balance at OpenAI, but at some providers only a per-minute limit.
    Quota,
    NoModel,
    NotOrdered,
    BadKey,
    NotFound,
    RateLimited,
    ServerError,
    Other,
}

impl Cause {
    /// A failure partway through an answer has no status: 0.
    pub(crate) fn of(status: u16, code: Option<&str>, reply: &str) -> Cause {
        let reply = format!("{} {reply}", code.unwrap_or_default()).to_lowercase();
        let says = |words: &[&str]| words.iter().any(|w| reply.contains(w));
        if says(&[
            "context_length",
            "context length",
            "maximum context",
            "too many tokens",
            "上下文长度",
        ]) {
            Cause::TooLong
        } else if status == 402
            || says(&[
                "insufficient balance",
                "insufficient credits",
                "credit balance",
                "余额不足",
                "额度用完",
                "insufficient funds",
                "insufficient account balance",
                "余额已耗尽",
                "欠费",
                "arrearage",
                "account is in good standing",
                "overdue",
                "free trial quota",
            ])
        {
            Cause::NoBalance
        } else if says(&[
            "insufficient_quota",
            "quota exceeded",
            "exceeded your current quota",
            "exceeded_current_quota",
        ]) {
            Cause::Quota
        } else if says(&[
            "model_not_found",
            "model not found",
            "model does not exist",
            "模型不存在",
        ]) {
            Cause::NoModel
        // Bailian and Ark answer 429 when the service was never ordered or a limit the user set
        // there is reached.
        } else if says(&[
            "commoditynotpurchased",
            "budgetlimitexceeded",
            "setlimitexceeded",
        ]) {
            Cause::NotOrdered
        } else {
            match status {
                401 | 403 => Cause::BadKey,
                404 => Cause::NotFound,
                429 => Cause::RateLimited,
                500.. => Cause::ServerError,
                _ => Cause::Other,
            }
        }
    }

    fn failure(self, reply: &str) -> Failure {
        match self {
            Cause::TooLong => Failure::new(TOO_LONG),
            Cause::NoBalance => Failure::new(NO_BALANCE),
            Cause::NoModel => Failure::new(NO_MODEL),
            Cause::NotOrdered => Failure::new(NOT_ORDERED),
            Cause::BadKey => {
                Failure::new("API Key 不正确或无权限。请检查 API Key 是否填错、是否已停用。")
            }
            Cause::NotFound => Failure::new(
                "服务商找不到这个模型或接口。请检查模型名是否正确；自定义的接口地址一般以 /v1 结尾。",
            ),
            Cause::Quota | Cause::RateLimited => {
                Failure::new("请求太频繁或额度已用完。请稍后再试，或到服务商网站查看额度。")
            }
            Cause::ServerError => Failure::unrecognized("服务商出错了。请稍后再试。", reply),
            Cause::Other => Failure::unrecognized(
                "服务商拒绝了请求。请检查模型名是否填错、这个模型是否支持对话。",
                reply,
            ),
        }
    }
}
