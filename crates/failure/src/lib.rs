//! A failure as the user sees it, and how outside words are cleaned before they leave the
//! computer (docs/development.md#错误和日志).

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// The most of a server's, provider's, website's or the system's words kept, in characters.
const DIAGNOSTIC_CHARS: usize = 500;

/// Why something failed, as the user is shown it. `details` holds outside words the app did not
/// recognize, for the user to make sense of or show to someone who can help.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct Failure {
    pub message: String,
    pub details: Option<String>,
}

impl Failure {
    /// A failure the app recognized: the message says all there is.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            details: None,
        }
    }

    /// A failure the app did not recognize, with the outside words, already without secrets.
    pub fn unrecognized(message: impl Into<String>, words: &str) -> Self {
        Self {
            message: message.into(),
            details: Some(scrub(&diagnostic(words))),
        }
    }
}

/// Outside words for the log, cut to a length that still shows what was said.
pub fn diagnostic(words: &str) -> String {
    match words.char_indices().nth(DIAGNOSTIC_CHARS) {
        Some((end, _)) => format!("{}…", &words[..end]),
        None => words.to_owned(),
    }
}

/// For text that leaves the computer, such as exported logs and details: an email address keeps
/// its first character and domain, the home directory, which names the user, becomes `~`, and a
/// web address loses what follows `?` or `#`.
pub fn scrub(text: &str) -> String {
    scrubbed(text, std::env::home_dir().as_deref())
}

static URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\b(https?://[^\s?#"'<>(){}\[\]]+)[?#][^\s"'<>(){}\[\]]*"#)
        .unwrap_or_else(|e| unreachable!("a fixed pattern: {e}"))
});

static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b([A-Za-z0-9._%+-])[A-Za-z0-9._%+-]*@([A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+)")
        .unwrap_or_else(|e| unreachable!("a fixed pattern: {e}"))
});

fn scrubbed(text: &str, home: Option<&Path>) -> String {
    let mut text = text.to_owned();
    // A home of "/" would take every path with it.
    if let Some(home) = home.map(|h| h.to_string_lossy()).filter(|h| h.len() > 1) {
        text = text.replace(home.as_ref(), "~");
        // Windows paths are written with doubled backslashes when logged with `{:?}`.
        if home.contains('\\') {
            text = text.replace(&home.replace('\\', r"\\"), "~");
        }
    }
    let text = URL.replace_all(&text, "$1");
    EMAIL.replace_all(&text, "$1***@$2").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubs_addresses_home_and_query_strings() {
        let text = "550 5.1.1 <wang.jianguo@example.edu.cn>: user unknown\n\
            open /Users/zhang/Documents/简历.pdf: denied\n\
            GET https://school.example.edu/list?token=abc#top 404\n\
            https://school.example.edu/teacher/1 ok";
        let scrubbed = scrubbed(text, Some(Path::new("/Users/zhang")));
        assert_eq!(
            scrubbed,
            "550 5.1.1 <w***@example.edu.cn>: user unknown\n\
            open ~/Documents/简历.pdf: denied\n\
            GET https://school.example.edu/list 404\n\
            https://school.example.edu/teacher/1 ok"
        );
        assert_eq!(super::scrubbed(&scrubbed, None), scrubbed);
    }

    #[test]
    fn scrubs_a_windows_home_however_it_was_written() {
        let text = r#"open C:\Users\zhang\a.pdf; path: "C:\\Users\\zhang\\a.pdf""#;
        assert_eq!(
            scrubbed(text, Some(Path::new(r"C:\Users\zhang"))),
            r#"open ~\a.pdf; path: "~\\a.pdf""#
        );
    }

    #[test]
    fn keeps_the_start_of_long_words() {
        let words = "错".repeat(DIAGNOSTIC_CHARS + 1);
        let kept = diagnostic(&words);
        assert_eq!(kept.chars().count(), DIAGNOSTIC_CHARS + 1);
        assert!(kept.ends_with('…'));
        assert_eq!(diagnostic("short"), "short");
        let failure = Failure::unrecognized("出错了", "<li@example.com> said no");
        assert_eq!(
            failure.details.as_deref(),
            Some("<l***@example.com> said no")
        );
    }
}
