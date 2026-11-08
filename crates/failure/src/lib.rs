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

/// Why reading or writing a file failed, after what failed (`what`, such as "日志未能导出").
pub fn file(what: &str, e: &std::io::Error) -> Failure {
    use std::io::ErrorKind;
    let reason = match e.kind() {
        ErrorKind::NotFound => "找不到这个文件或文件夹，可能已被移动或删除",
        // Windows refuses to replace a file another program has open, such as a table in Excel.
        ErrorKind::PermissionDenied | ErrorKind::ResourceBusy => {
            "没有权限读写这个位置，或文件正被其他程序打开。请关闭打开它的程序，或换一个位置后重试。"
        }
        ErrorKind::StorageFull | ErrorKind::QuotaExceeded => "磁盘空间不足，请清理出空间后重试",
        ErrorKind::ReadOnlyFilesystem => "这个位置只能读取，请换一个位置",
        // FAT32, common on USB drives, holds no file over 4 GB.
        ErrorKind::FileTooLarge => "文件超过了这个磁盘允许的大小，请换一个磁盘",
        _ => return Failure::unrecognized(format!("{what}，请重试"), &e.to_string()),
    };
    Failure::new(format!("{what}：{reason}"))
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
    static HOME: LazyLock<Option<Regex>> =
        LazyLock::new(|| std::env::home_dir().as_deref().and_then(home));
    scrubbed(text, HOME.as_ref())
}

/// The home directory however it is written: in either case, as Windows and macOS do not tell
/// them apart, and on Windows with backslashes, doubled ones as `{:?}` writes them, or slashes.
/// Only whole, so that /Users/li does not take /Users/lin with it.
fn home(home: &Path) -> Option<Regex> {
    let home = home.to_string_lossy();
    // A home of "/" would take every path with it.
    if home.len() <= 1 {
        return None;
    }
    let separator = if home.contains('\\') {
        r"(?:\\\\|\\|/)"
    } else {
        "/"
    };
    let parts: Vec<String> = home.split(['/', '\\']).map(regex::escape).collect();
    let pattern = format!(r"(?i){}([^\w.-]|$)", parts.join(separator));
    Some(Regex::new(&pattern).unwrap_or_else(|e| unreachable!("an escaped path: {e}")))
}

static URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\b(https?://[^\s?#"'<>(){}\[\]]+)[?#][^\s"'<>(){}\[\]]*"#)
        .unwrap_or_else(|e| unreachable!("a fixed pattern: {e}"))
});

/// The characters an address may have before its @, but for those that more often stand around
/// one in a log, such as double quotes, slashes, = and braces. An apostrophe, as in o'neil, and *
/// are taken inside it, but do not start it: they also quote or stress one.
static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"([A-Za-z0-9!#$%&+^_`~-])[A-Za-z0-9.!#$%&'*+^_`~-]*@([A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+)",
    )
    .unwrap_or_else(|e| unreachable!("a fixed pattern: {e}"))
});

fn scrubbed(text: &str, home: Option<&Regex>) -> String {
    let text = match home {
        Some(home) => home.replace_all(text, "~$1"),
        None => text.into(),
    };
    let text = URL.replace_all(&text, "$1");
    EMAIL.replace_all(&text, "$1***@$2").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubs_addresses_home_and_query_strings() {
        let text = "550 5.1.1 <wang.jianguo@example.edu.cn>: user unknown\n\
            from alice!smith@example.com, o'neil@example.com\n\
            to='li@example.com' **zhao@example.com** key=sun@example.com\n\
            open /Users/zhang/Documents/简历.pdf: denied, /users/ZHANG too, not /Users/zhang2\n\
            GET https://school.example.edu/list?token=abc#top 404\n\
            https://school.example.edu/teacher/1 ok";
        let scrubbed = scrubbed(text, home(Path::new("/Users/zhang")).as_ref());
        assert_eq!(
            scrubbed,
            "550 5.1.1 <w***@example.edu.cn>: user unknown\n\
            from a***@example.com, o***@example.com\n\
            to='l***@example.com' **z***@example.com** key=s***@example.com\n\
            open ~/Documents/简历.pdf: denied, ~ too, not /Users/zhang2\n\
            GET https://school.example.edu/list 404\n\
            https://school.example.edu/teacher/1 ok"
        );
        assert_eq!(super::scrubbed(&scrubbed, None), scrubbed);
    }

    #[test]
    fn scrubs_a_windows_home_however_it_was_written() {
        let text =
            r#"open C:\Users\zhang\a.pdf; path: "C:\\Users\\zhang\\a.pdf"; c:/users/Zhang/b"#;
        assert_eq!(
            scrubbed(text, home(Path::new(r"C:\Users\zhang")).as_ref()),
            r#"open ~\a.pdf; path: "~\\a.pdf"; ~/b"#
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
