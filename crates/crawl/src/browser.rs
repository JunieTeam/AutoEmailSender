//! A desktop browser supplies HTML; parsing and claim ownership stay in crawl.
use std::{future::Future, pin::Pin};

use crate::{PageTarget, store::PageStep};
use failure::Failure;

pub struct RenderedPage {
    pub url: String,
    pub html: String,
}

pub trait Browser: Send + Sync {
    /// Dropping the future must close its window, including while waiting for a slot.
    fn render<'a>(
        &'a self,
        target: &'a PageTarget,
        steps: &'a [PageStep],
    ) -> Pin<Box<dyn Future<Output = Result<RenderedPage, BrowserError>> + Send + 'a>>;
}

#[derive(Debug)]
pub enum BrowserError {
    TimedOut,
    Failed(Failure),
}

impl BrowserError {
    pub fn failure(self) -> Failure {
        match self {
            Self::TimedOut => Failure::new("网页在浏览器里 20 秒内未加载完成"),
            Self::Failed(reason) => reason,
        }
    }
}
