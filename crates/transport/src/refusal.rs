//! What a mail server turning mail down means, from its reply when sending or the diagnostic a
//! bounce carries. Markers are lowercase and taken from real replies (V2 and the providers'
//! documentation); the first kind that matches wins.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The address mail goes from is not the one logged in.
    Sender,
    /// The server does not pass mail on for this account.
    Relay,
    /// The account is disabled, locked, or may not send.
    Account,
    MailboxFull,
    TooLarge,
    NoSuchRecipient,
    /// Spam filters or the receiving side's policy.
    Rejected,
}

const KINDS: [(Refusal, &[&str]); 7] = [
    (
        Refusal::Sender,
        &[
            "mail from must equal authorized user",
            "must be same as authorization user",
            "sender address rejected",
            "sender not authorized",
            "not authorized to send",
            "not owned by user",
            "send as denied",
            "envelope sender",
            "dt:sum",
            "发件人地址不匹配",
            "发件人不一致",
            "无权代发",
        ],
    ),
    (
        Refusal::Relay,
        &[
            "relay access denied",
            "relaying denied",
            "relay not permitted",
            "unable to relay",
            "relay denied",
            "中继被拒绝",
        ],
    ),
    (
        Refusal::Account,
        &[
            "account disabled",
            "account suspended",
            "account locked",
            "user disabled",
            "user suspended",
            "mailbox locked",
            "user has no permission",
            "permission denied",
            "账号被禁用",
            "账号已停用",
            "账号已冻结",
            "账号被锁定",
            "没有发信权限",
        ],
    ),
    (
        Refusal::MailboxFull,
        &[
            "5.2.2",
            "mailbox full",
            "mailbox is full",
            "mailbox quota exceeded",
            "recipient quota exceeded",
            "over quota",
            "quota exceeded",
            "insufficient storage",
            "收件箱已满",
            "邮箱已满",
            "空间不足",
        ],
    ),
    (
        Refusal::TooLarge,
        &[
            "5.3.4",
            "message too large",
            "message size exceeds",
            "message size limit",
            "maximum message size",
            "size limit exceeded",
            "邮件过大",
            "附件过大",
            "超过邮件大小",
        ],
    ),
    (
        Refusal::NoSuchRecipient,
        &[
            "5.1.1",
            "5.1.0",
            "user unknown",
            "unknown user",
            "no such user",
            "unknown recipient",
            "invalid recipient",
            "recipient not found",
            "user not found",
            "mailbox not found",
            "recipient address rejected",
            "address rejected",
            "mailbox unavailable",
            "does not exist",
            "doesn't exist",
            "不存在",
            "无此用户",
            "无效收件人",
        ],
    ),
    (
        Refusal::Rejected,
        &[
            "5.7.",
            "spam",
            "junk mail",
            "blacklist",
            "blocked",
            "policy",
            "content rejected",
            "message rejected",
            "mi:spf",
            "mi:dma",
            "spf check",
            "dmarc",
            "dkim",
            "垃圾邮件",
            "反垃圾",
            "安全策略",
            "拒收",
            "拒绝",
        ],
    ),
];

impl Refusal {
    /// What the server's words say; None when they fit no kind.
    pub fn of(words: &str) -> Option<Refusal> {
        let words = words.to_lowercase();
        KINDS
            .iter()
            .find(|(_, markers)| markers.iter().any(|m| words.contains(m)))
            .map(|(kind, _)| *kind)
    }

    /// Whether the words say this kind, whatever other kinds they also say.
    pub fn said_in(self, words: &str) -> bool {
        let words = words.to_lowercase();
        KINDS
            .iter()
            .any(|(kind, markers)| *kind == self && markers.iter().any(|m| words.contains(m)))
    }

    pub fn message(self) -> &'static str {
        match self {
            Refusal::Sender => {
                "发件地址与登录的账号不一致，服务器不允许代发。请把发件邮箱的地址改成登录用的地址。"
            }
            Refusal::Relay => {
                "服务器拒绝代发这封邮件。请检查发信服务器的地址、端口和加密方式是否是这个邮箱的。"
            }
            Refusal::Account => {
                "发件账号已停用、被锁定，或未开启发信权限。请登录网页邮箱检查账号状态，并开启 SMTP 服务。"
            }
            Refusal::MailboxFull => {
                "对方邮箱已满，暂时收不了邮件。可过几天再发，或改发到导师的其他邮箱。"
            }
            Refusal::TooLarge => "邮件超过了服务器允许的大小。请减少或压缩附件后重试。",
            Refusal::NoSuchRecipient => "收件地址不存在。请检查导师的邮箱是否填错。",
            Refusal::Rejected => {
                "邮件被收件方的服务器拒收，可能被当成了垃圾邮件。请修改邮件内容（比如减少链接和附件）后重试，\
                或改发到导师的其他邮箱。"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Refusal;

    #[test]
    fn real_replies_fit_their_kind() {
        for (words, kind) in [
            (
                "553 Mail from must equal authorized user",
                Some(Refusal::Sender),
            ),
            ("550 User has no permission", Some(Refusal::Account)),
            ("552 5.2.2 Mailbox full", Some(Refusal::MailboxFull)),
            (
                "552 5.3.4 Message size exceeds fixed maximum message size",
                Some(Refusal::TooLarge),
            ),
            (
                "550 5.1.1 <wang@example.edu.cn>: Recipient address rejected: User unknown",
                Some(Refusal::NoSuchRecipient),
            ),
            (
                "554 DT:SPM 163 smtp14,EsCowA spam message",
                Some(Refusal::Rejected),
            ),
            (
                "550 5.7.1 Message rejected by policy",
                Some(Refusal::Rejected),
            ),
            ("451 Temporary local problem", None),
        ] {
            assert_eq!(Refusal::of(words), kind, "{words}");
        }
    }
}
