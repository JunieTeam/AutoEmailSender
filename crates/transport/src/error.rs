use std::io;

use failure::Failure;

use crate::pause::{Effect, PauseKind, classify};
use crate::refusal::Refusal;

/// Why talking to a mail server failed. Displays in English for the log, without the server's
/// words, which may quote the recipient or the letter; `failure` is for the user and `diagnostic`
/// for the log.
#[derive(Debug)]
pub enum MailError {
    /// The server's name could not be looked up.
    Resolve {
        host: String,
        source: io::Error,
    },
    /// The server did not take the connection.
    Connect {
        host: String,
        port: u16,
        source: io::Error,
    },
    /// The system proxy did not take the connection.
    ProxyDown {
        proxy: String,
        source: io::Error,
    },
    /// The proxy took the connection but could not reach the server.
    ProxyFailed {
        proxy: String,
        host: String,
        port: u16,
        reason: String,
    },
    TimedOut {
        host: String,
        port: u16,
    },
    /// The encrypted connection could not be set up.
    Tls {
        host: String,
        source: io::Error,
    },
    /// The server turned down the login, with its reply.
    Login {
        reply: String,
        smtp_code: Option<u16>,
    },
    /// The server turned down some other request, with its reply.
    Refused {
        reply: String,
        smtp_code: Option<u16>,
    },
    /// The connection broke off, or the server answered in a way that makes no sense.
    Broken(String),
    /// The server answered in a way the mail library cannot read. Its own wording quotes the reply,
    /// so none is kept.
    Unreadable,
    /// An address mail cannot be sent to.
    Recipient(String),
}

impl std::fmt::Display for MailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MailError::Resolve { host, source } => write!(f, "look up {host}: {source}"),
            MailError::Connect { host, port, source } => {
                write!(f, "connect to {host}:{port}: {source}")
            }
            MailError::ProxyDown { proxy, source } => {
                write!(f, "connect to proxy {proxy}: {source}")
            }
            MailError::ProxyFailed {
                proxy,
                host,
                port,
                reason,
            } => write!(f, "proxy {proxy} to {host}:{port}: {reason}"),
            MailError::TimedOut { host, port } => write!(f, "{host}:{port} timed out"),
            MailError::Tls { host, source } => write!(f, "tls with {host}: {source}"),
            MailError::Login { reply, smtp_code } => {
                write!(f, "login refused")?;
                codes(f, reply, *smtp_code)
            }
            MailError::Refused { reply, smtp_code } => {
                write!(f, "refused")?;
                codes(f, reply, *smtp_code)
            }
            MailError::Broken(e) => write!(f, "{e}"),
            MailError::Unreadable => write!(f, "unreadable reply"),
            MailError::Recipient(_) => write!(f, "unusable recipient"),
        }
    }
}

/// An SMTP refusal's codes, such as ` (550 5.1.1)`; an IMAP one has none.
fn codes(f: &mut std::fmt::Formatter<'_>, reply: &str, smtp_code: Option<u16>) -> std::fmt::Result {
    let Some(code) = smtp_code else {
        return Ok(());
    };
    // Worded by lettre as "permanent error (550): 5.1.1 ...", the enhanced code first if any.
    let first = reply
        .split_once("): ")
        .and_then(|(_, words)| words.split_whitespace().next());
    let number =
        |part: &str| (1..=3).contains(&part.len()) && part.bytes().all(|b| b.is_ascii_digit());
    match first.filter(|c| c.split('.').count() == 3 && c.split('.').all(number)) {
        Some(enhanced) => write!(f, " ({code} {enhanced})"),
        None => write!(f, " ({code})"),
    }
}

impl std::error::Error for MailError {}

impl MailError {
    /// A refusal taken as one of the login.
    pub fn login(self) -> MailError {
        match self {
            MailError::Refused { reply, smtp_code } => MailError::Login { reply, smtp_code },
            e => e,
        }
    }

    /// Without the mailbox's secret, should the server quote it back: what the server says reaches
    /// the user, Agents and the mailbox's saved error.
    pub fn without(self, secret: &str) -> MailError {
        let hide = |words: String| words.replace(secret, "***");
        match self {
            MailError::Login { reply, smtp_code } => MailError::Login {
                reply: hide(reply),
                smtp_code,
            },
            MailError::Refused { reply, smtp_code } => MailError::Refused {
                reply: hide(reply),
                smtp_code,
            },
            e => e,
        }
    }

    /// What happened and what to do about it; the server's words only when no case fits them.
    pub fn failure(&self) -> Failure {
        match self {
            MailError::Resolve { host, .. } => Failure::new(format!(
                "找不到服务器 {host}。请检查服务器地址是否正确、网络是否连通。"
            )),
            MailError::Connect { host, port, .. } => Failure::new(format!(
                "无法连接 {host} 的 {port} 端口。请检查服务器地址和端口是否正确、网络是否连通。"
            )),
            MailError::ProxyDown { proxy, .. } => Failure::new(format!(
                "无法连接系统代理 {proxy}。请确认代理软件正在运行，或关闭系统代理。"
            )),
            MailError::ProxyFailed {
                proxy, host, port, ..
            } => Failure::new(format!(
                "系统代理 {proxy} 未能连接 {host} 的 {port} 端口。请检查服务器地址和端口，或更换代理节点。"
            )),
            MailError::TimedOut { host, port } => Failure::new(format!(
                "{host} 的 {port} 端口长时间无响应。请检查服务器地址和端口；在国内连接 Gmail \
                等国外邮箱需要开启代理。"
            )),
            MailError::Tls { host, source } if net::untrusted(source) => Failure::new(format!(
                "{host} 的证书不受信任，连接可能被拦截。请更换网络后重试。"
            )),
            MailError::Tls { host, .. } => Failure::new(format!(
                "与 {host} 建立加密连接失败。请检查加密方式和端口是否对应：SSL/TLS \
                一般用 465、993 端口，STARTTLS 一般用 587、143 端口。"
            )),
            // Checking the code or trying again at once would only keep the limit on.
            MailError::Login { .. } | MailError::Refused { .. }
                if matches!(classify(self), Effect::Pause(PauseKind::Throttled(_))) =>
            {
                Failure::new(
                    "服务商限制了登录或发信的频率。请稍后再试，无需修改授权码；\
                    短时间内反复尝试会延长限制。",
                )
            }
            MailError::Login { reply, .. } => login(reply),
            MailError::Refused { reply, .. } => match Refusal::of(reply) {
                Some(refusal) => Failure::new(refusal.message()),
                None => Failure::unrecognized("服务器拒绝了请求", reply),
            },
            MailError::Broken(e) => Failure::unrecognized("和服务器通信出错。请稍后再试。", e),
            MailError::Unreadable => Failure::new("服务器的回复无法读取。请稍后再试。"),
            MailError::Recipient(address) => Failure::new(format!(
                "收件地址 {address} 无法使用。请检查导师的邮箱是否填错。"
            )),
        }
    }

    /// The server's words, for the log.
    pub fn diagnostic(&self) -> Option<String> {
        match self {
            MailError::Login { reply, .. } | MailError::Refused { reply, .. } => {
                Some(failure::diagnostic(reply))
            }
            MailError::ProxyFailed { reason, .. } => Some(failure::diagnostic(reason)),
            MailError::Broken(e) => Some(failure::diagnostic(e)),
            _ => None,
        }
    }
}

/// Why a login was turned down, by what the server says; markers from real replies.
fn login(reply: &str) -> Failure {
    let words = reply.to_lowercase();
    let says = |markers: &[&str]| markers.iter().any(|m| words.contains(m));
    if says(&["application-specific password", "5.7.9"]) {
        Failure::new(
            "这个邮箱要用应用专用密码登录。请到邮箱的安全设置里生成应用专用密码，填到授权码一栏。",
        )
    } else if says(&["web browser", "5.7.14"]) {
        Failure::new(
            "邮箱服务商要求先在网页上登录这个邮箱。请用浏览器登录邮箱，按提示完成验证后再试。",
        )
    } else if says(&["unsafe login"]) {
        Failure::new(
            "邮箱服务商的安全策略拦截了这次登录。请登录网页邮箱，按提示解除限制或开启客户端登录后再试。",
        )
    } else if says(&["authorization code", "authorized code", "授权码"]) {
        Failure::new(
            "这个邮箱要用授权码登录，不能用邮箱密码。请到邮箱设置里开启 IMAP/SMTP 服务、生成授权码后填入。",
        )
    } else if let Some(Refusal::Account) = Refusal::of(reply) {
        Failure::new(Refusal::Account.message())
    } else {
        Failure::unrecognized(
            "登录被拒绝。请检查授权码或密码是否正确，邮箱设置中是否已开启 IMAP/SMTP 服务；\
            学校和公司的邮箱可能被管理员关闭了客户端登录，需联系管理员。",
            reply,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::MailError;
    use crate::refusal::Refusal;

    #[test]
    fn a_refusal_quoting_the_recipient_logs_only_its_codes() {
        let refused = MailError::Refused {
            reply: "permanent error (550): 5.1.1 <wang@example.edu.cn>: Recipient address rejected"
                .into(),
            smtp_code: Some(550),
        };
        assert_eq!(refused.to_string(), "refused (550 5.1.1)");
        assert!(
            refused
                .diagnostic()
                .unwrap()
                .contains("wang@example.edu.cn")
        );
        assert_eq!(
            refused.failure().message,
            Refusal::NoSuchRecipient.message()
        );
        let unusable = MailError::Recipient("wang@example.edu.cn".into());
        assert!(!unusable.to_string().contains("wang"), "{unusable}");
    }

    #[test]
    fn a_refused_login_quoting_the_secret_does_not_pass_it_on() {
        let refused = MailError::Refused {
            reply: "permanent error (535): 5.7.8 bad secret abcd-1234".into(),
            smtp_code: Some(535),
        }
        .login()
        .without("abcd-1234");
        assert!(matches!(refused, MailError::Login { .. }), "{refused:?}");
        let failure = refused.failure();
        assert!(!format!("{failure:?}").contains("abcd-1234"), "{failure:?}");
        assert!(failure.details.is_some(), "{failure:?}");
    }
}
