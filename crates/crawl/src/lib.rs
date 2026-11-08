//! Public school pages, structured evidence, ownership checks, and the page ledger.

pub mod browser;
use failure::Failure;

mod email;
mod embedded;
mod extract;
pub mod fetch;
mod ownership;
mod page;
mod pagination;
mod pdf;
pub mod queue;
pub mod review;
pub mod runner;
pub mod store;
pub mod submit;
mod url;

pub use ownership::owns_email;
pub use page::{
    BrowserReason, EmailEvidence, EmailLocation, Link, Material, Pagination, Unit, parse,
    parse_rendered,
};
pub use url::{PageTarget, PageUrl, Scope};

pub fn span(id: i64) -> tracing::Span {
    tracing::info_span!(parent: None, "crawl", id)
}

#[derive(Debug)]
pub enum CrawlError {
    Url,
    OutsideSite,
    Redirect,
    TooLarge,
    TimedOut,
    Status(u16),
    /// The website asked to be left alone for this long.
    SlowDown(std::time::Duration),
    ContentType,
    /// A PDF whose text cannot be read, with why.
    Document(String),
    Http(reqwest::Error),
    Worker(tokio::task::JoinError),
}

impl std::fmt::Display for CrawlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Url => f.write_str("invalid or non-public page URL"),
            Self::OutsideSite => f.write_str("page is outside the allowed website"),
            Self::Redirect => f.write_str("invalid or excessive redirects"),
            Self::TooLarge => f.write_str("page exceeds the size limit"),
            Self::TimedOut => f.write_str("page request timed out"),
            Self::Status(code) => write!(f, "page returned HTTP {code}"),
            Self::SlowDown(wait) => write!(f, "website asked to wait {}s", wait.as_secs()),
            Self::ContentType => f.write_str("response is not an HTML or text page"),
            Self::Document(reason) => write!(f, "read PDF: {reason}"),
            Self::Http(error) => write!(f, "fetch page: {error}"),
            Self::Worker(error) => write!(f, "page worker: {error}"),
        }
    }
}

impl std::error::Error for CrawlError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Http(e) => Some(e),
            Self::Worker(e) => Some(e),
            _ => None,
        }
    }
}

impl CrawlError {
    /// What happened and what to do about it; the library's words only when no case fits them.
    pub fn failure(&self) -> Failure {
        match self {
            Self::Url => Failure::new("网址无法使用。请填写公开的 http 或 https 网页地址。"),
            Self::OutsideSite => Failure::new("页面超出了这次整理允许访问的网站"),
            Self::Redirect => Failure::new("网页的跳转地址无效，或跳转次数过多。请检查原网址。"),
            Self::TooLarge => Failure::new("网页过大，无法读取。请改用具体的名单页或导师个人页。"),
            Self::TimedOut => Failure::new("读取网页超时。请检查网络，稍后重试。"),
            Self::Status(404 | 410) => {
                Failure::new("网页不存在。请在浏览器中打开网址检查，或改用别的页面。")
            }
            Self::Status(401 | 403) => {
                Failure::new("网站拒绝访问，可能要登录或只对校内网络开放。请改用公开的页面。")
            }
            Self::Status(500..=599) => Failure::new("网站出错了。请稍后重试。"),
            Self::Status(code) => Failure::unrecognized(
                "网站没有正常返回网页。请稍后重试，或在浏览器中检查网址。",
                &format!("HTTP {code}"),
            ),
            Self::SlowDown(_) => Failure::new("网站要求放慢访问。请稍后重试。"),
            Self::ContentType => Failure::new("这个地址返回的不是网页。请改用名单页或导师个人页。"),
            Self::Document(reason) => {
                Failure::unrecognized("这个 PDF 文件读不出文字，可能已损坏或加密", reason)
            }
            Self::Http(e) if net::untrusted(e) => {
                Failure::new("网站的证书不受信任，连接可能被拦截。请更换网络后重试。")
            }
            Self::Http(e) if e.is_connect() => {
                Failure::new("无法连接网站。请检查网址、网络和系统代理，稍后重试。")
            }
            Self::Http(e) => {
                Failure::unrecognized("无法打开网页。请检查网络和系统代理，稍后重试。", &chain(e))
            }
            Self::Worker(_) => Failure::new(
                "读取网页内容失败，请重试；仍然出错的，可在电脑上的 设置 › 关于和日志 里反馈问题",
            ),
        }
    }

    /// The library's words, for the log.
    pub fn diagnostic(&self) -> Option<String> {
        match self {
            Self::Document(reason) => Some(failure::diagnostic(reason)),
            Self::Http(e) => Some(failure::diagnostic(&chain(e))),
            _ => None,
        }
    }
}

/// An error with its causes: reqwest's own words are only "error sending request".
fn chain(e: &(dyn std::error::Error + 'static)) -> String {
    let mut words = e.to_string();
    let mut next = e.source();
    while let Some(e) = next {
        words += &format!(": {e}");
        next = e.source();
    }
    words
}

pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

pub mod enrich;
