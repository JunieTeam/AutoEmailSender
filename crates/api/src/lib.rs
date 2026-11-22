//! The operations the UI, the phone and agents call, and the one dispatcher that runs them all
//! (docs/design.md#桌面端). Input is validated here, when it is deserialized.

pub mod agent;
pub mod agent_settings;
pub mod phones;
use agent::{
    answer_agent_request, describe_operation, get_agent_access, list_agent_requests,
    list_operations, set_agent_access,
};
use agent_settings::*;
use phones::*;
mod accounts;
pub mod libraries;
pub mod logs;
pub mod notifications;
use libraries::*;
pub mod backup;
use accounts::*;
mod cloud_backups;
use cloud_backups::*;
pub mod cloud_send;
use cloud_send::{cloud_send_preview, hand_off_to_cloud, withdraw_from_cloud};
mod crawling;
mod fact_check;
mod feedback;
mod mail_domains;
use fact_check::*;
use feedback::{
    FeedbackInput, FeedbackPreview, get_feedback_log, preview_feedback, submit_feedback,
};
pub use mail_domains::MailDomains;
mod correspondence;
mod first_emails;
mod follow_ups;
mod mailboxes;
mod match_analysis;
mod materials;
mod mentors;
mod models;
mod profiles;
mod receiving;
mod replies;
mod reply_outcomes;
pub mod startup;
mod statistics;
mod tags;
mod tasks;
mod templates;
mod todos;
pub mod tracking;
mod ui_errors;
pub mod updater;
mod writing_turns;
use correspondence::*;
use crawling::*;
use first_emails::*;
use follow_ups::*;
use mailboxes::*;
use match_analysis::*;
use materials::*;
use mentors::*;
pub use mentors::{ExportMentors, TableFormat, export_mentors};
use models::*;
use profiles::*;
use receiving::*;
use replies::*;
use reply_outcomes::*;
use statistics::*;
use std::collections::BTreeMap;
use tags::*;
use tasks::*;
use templates::*;
use todos::*;
use tracking::{get_tracking, set_tracking};
use ui_errors::{UiErrorReport, log_ui_error};
use writing_turns::{Writing, WritingRequest, cancel_writing};

use base64::prelude::{BASE64_STANDARD, Engine};
use failure::Failure;
use model::usage::About;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use ts_rs::{Config, TS, TypeVisitor};

pub struct Context {
    pub account: account::Client,
    pub library: library::sync::Synchronizer,
    pub agent_settings: agent_settings::Settings,
    pub agent_access: agent::Access,
    pub phones: phones::Phones,
    pub writes: db::Writes,
    pub db: SqlitePool,
    pub net: net::Net,
    pub sender: send::Sender,
    pub receiver: receive::Receiver,
    pub matcher: matching::Matcher,
    pub reply_processor: receive::outcome::Processor,
    pub todo_reminders: receive::reminders::Scheduler,
    pub crawler: crawl::runner::Crawler,
    pub generator: task::generation::Generator,
    pub turns: model::background::Turns,
    pub writings: writing_turns::Writings,
    pub mail_domains: MailDomains,
    pub cloud_backup: backup::cloud::CloudBackup,
    pub tracking: tracking::Tracker,
    pub cloud_send: cloud_send::CloudSend,
    /// Where material files are kept.
    pub materials: std::path::PathBuf,
    /// Where the desktop app keeps its log files.
    pub logs: std::path::PathBuf,
    pub feedback: feedback::Feedback,
}

impl Context {
    /// `data` is the data directory.
    pub fn new(db: SqlitePool, net: net::Net, data: &std::path::Path) -> Self {
        let writes = db::Writes::default();
        Self {
            account: account::Client::new(&net),
            library: Default::default(),
            agent_settings: Default::default(),
            agent_access: Default::default(),
            phones: Default::default(),
            crawler: crawl::runner::Crawler::new(writes.clone()),
            writes,
            db,
            net,
            cloud_backup: backup::cloud::CloudBackup::new(data),
            tracking: Default::default(),
            cloud_send: Default::default(),
            materials: data.join("materials"),
            logs: data.join("logs"),
            feedback: Default::default(),
            sender: Default::default(),
            receiver: Default::default(),
            matcher: Default::default(),
            reply_processor: Default::default(),
            todo_reminders: Default::default(),
            generator: Default::default(),
            turns: Default::default(),
            writings: Default::default(),
            mail_domains: Default::default(),
        }
    }
}

/// What the UI shows when an operation fails (docs/development.md#错误和日志).
#[derive(Debug, Serialize, TS)]
pub struct CallError {
    pub code: ErrorCode,
    pub message: String,
    /// The outside words the app did not recognize, shown folded under the message.
    pub details: Option<String>,
    /// What finding out why needs, such as the diagnostic and the records involved; only logged.
    #[serde(skip)]
    #[ts(skip)]
    log: Option<String>,
}

/// Who called an operation, for the log.
#[derive(Debug, Clone, Copy)]
pub enum Caller {
    Window,
    Phone,
    Agent,
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The caller sent something the operation does not accept; a bug in the caller.
    BadInput,
    /// What the caller asked for no longer exists, such as a mentor deleted since the page opened.
    NotFound,
    /// What the user filled in cannot be saved; the message says what to change.
    Invalid,
    /// A service outside the app, such as a mail server, did not do what was asked; the message
    /// says why and what to do.
    External,
    /// The official server no longer accepts this computer's login; the account now reads as
    /// expired.
    SignedOut,
    NeedsAgreement,
    /// Logging in to an account in its cooling-off period; logging in with `restore` keeps it.
    AccountDeleting,
    Internal,
}

impl CallError {
    fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        CallError {
            code,
            message: message.into(),
            details: None,
            log: None,
        }
    }

    fn bad_input(e: impl std::fmt::Display) -> Self {
        Self::new(ErrorCode::BadInput, format!("参数不正确：{e}"))
    }

    /// What was asked cannot be done as it is; the message says what to change.
    pub fn invalid(message: String) -> Self {
        Self::new(ErrorCode::Invalid, message)
    }

    pub fn not_found(message: &str) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    /// A defect of the app's own, `e` saying what went wrong; the user is told to try again.
    pub fn internal(e: impl std::fmt::Display) -> Self {
        Self::new(ErrorCode::Internal, DEFECT).logging(e)
    }

    /// Something outside the app, a server or the system, did not do what was asked.
    pub fn external(failure: Failure, log: impl std::fmt::Display) -> Self {
        CallError {
            details: failure.details,
            ..Self::new(ErrorCode::External, failure.message).logging(log)
        }
    }

    /// With `log` for the log, after what it already has: the records involved, which a
    /// conversion from a lower error does not know, or the diagnostic.
    pub fn logging(mut self, log: impl std::fmt::Display) -> Self {
        self.log = Some(match self.log {
            Some(earlier) => format!("{log}: {earlier}"),
            None => log.to_string(),
        });
        self
    }

    /// Logs the error where it leaves the core or the app, once: the message only of an outside
    /// failure or a defect, as the others may quote what the user filled in.
    pub fn record(&self, op: &str, caller: Caller) {
        let code = &self.code;
        let log = self.log.as_deref();
        match code {
            ErrorCode::Internal => {
                tracing::error!(
                    op,
                    ?caller,
                    ?code,
                    message = self.message,
                    log,
                    "call failed"
                )
            }
            ErrorCode::External => {
                tracing::warn!(
                    op,
                    ?caller,
                    ?code,
                    message = self.message,
                    log,
                    "call failed"
                )
            }
            _ => tracing::info!(op, ?caller, ?code, log, "call failed"),
        }
    }
}

/// The code and what the log is told, for errors logged outside `record`.
impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.code)?;
        match &self.log {
            Some(log) => write!(f, ": {log}"),
            None => Ok(()),
        }
    }
}

const DEFECT: &str = "出错了，请重试；仍然出错的，可在电脑上的 设置 › 关于和日志 里反馈问题";

impl From<sqlx::Error> for CallError {
    fn from(e: sqlx::Error) -> Self {
        CallError::internal(format!("database: {e}"))
    }
}

/// Operations only the app's window may call: neither an Agent nor a phone decides who may connect,
/// answers an Agent's request or sends this computer's logs away.
pub(crate) fn window_only(op: &str) -> bool {
    matches!(
        op,
        "getAgentAccess"
            | "setAgentAccess"
            | "listAgentRequests"
            | "answerAgentRequest"
            | "getAgentCli"
            | "installAgentCli"
            | "uninstallAgentCli"
            | "previewMcpConfig"
            | "writeMcpConfig"
            | "mcpConfigText"
            | "getPhoneAccess"
            | "setPhoneAccess"
            | "addPhone"
            | "answerPhonePairing"
            | "cancelPhonePairing"
            | "listPhones"
            | "revokePhone"
            | "previewFeedback"
            | "getFeedbackLog"
            | "submitFeedback"
    )
}

/// Operations only the app's UI calls, on the computer or a phone: an Agent leaves out the request
/// that `cancelWriting` withdraws, and has no UI whose defects `logUiError` logs.
fn ui_only(op: &str) -> bool {
    window_only(op) || matches!(op, "cancelWriting" | "logUiError")
}

/// A change that found nothing to change, as when the thing is gone, as an error saying so.
fn require_applied(applied: bool, message: &str) -> Result<(), CallError> {
    if applied {
        Ok(())
    } else {
        Err(CallError::not_found(message))
    }
}

/// Runs one operation, its future boxed and built in a function of its own. Awaited in `call`,
/// every call would hold the largest operation; boxed there, unoptimized builds would still give
/// `call` stack for building every operation's future, and a few calls together overflowed a
/// Windows thread's stack in debug builds.
fn run<'a, I, O, F>(
    ctx: &'a Context,
    input: serde_json::Value,
    handler: impl FnOnce(&'a Context, I) -> F + 'a,
) -> std::pin::Pin<Box<impl Future<Output = Result<serde_json::Value, CallError>> + 'a>>
where
    I: serde::de::DeserializeOwned,
    O: serde::Serialize,
    F: Future<Output = Result<O, CallError>> + 'a,
{
    Box::pin(async move {
        let input =
            serde_path_to_error::deserialize::<_, I>(input).map_err(CallError::bad_input)?;
        serde_json::to_value(handler(ctx, input).await?).map_err(CallError::internal)
    })
}

macro_rules! operation_confirmation {
    (none) => {
        agent::OperationConfirmation::None
    };
    (confirm) => {
        agent::OperationConfirmation::Always
    };
    (confirm_if $condition:literal) => {
        agent::OperationConfirmation::Conditional($condition.into())
    };
}

/// Declares the catalog alongside the types and handlers, so all callers see the same operations.
macro_rules! operations {
    ($($group:literal {
        $(
            #[doc = $summary:literal]
            $(#[doc = $details:expr])*
            $op:literal [$confirmation:ident $($condition:literal)?]
                => $handler:ident($input:ty) -> $output:ty;
        )*
    })*) => {
        pub async fn call(
            ctx: &Context,
            op: &str,
            input: serde_json::Value,
        ) -> Result<serde_json::Value, CallError> {
            match op {
                $($($op => run::<$input, _, _>(ctx, input, $handler).await,)*)*
                _ => Err(CallError::bad_input(format!("不存在操作 {op}"))),
            }
        }

        fn operation_catalog() -> Vec<agent::OperationSummary> {
            vec![$($(
                agent::OperationSummary {
                    name: $op.into(),
                    summary: $summary.trim().into(),
                    group: $group.into(),
                    confirmation: operation_confirmation!($confirmation $($condition)?),
                },
            )*)*]
                .into_iter()
                .filter(|op| !ui_only(&op.name))
                .collect()
        }

        fn operation_description(name: &str) -> Result<agent::OperationDescription, CallError> {
            if ui_only(name) {
                return Err(CallError::invalid("这个操作只供应用界面使用".into()));
            }
            match name {
                $($($op => {
                    let cfg = Config::new().with_large_int("number");
                    let mut decls = BTreeMap::new();
                    collect::<$input>(&cfg, &mut decls);
                    collect::<$output>(&cfg, &mut decls);
                    let mut typescript = decls.values()
                        .map(|decl| format!("export {decl}\n\n"))
                        .collect::<String>();
                    typescript.push_str(&format!(
                        "export type Input = {};\nexport type Output = {};\n",
                        <$input>::name(&cfg),
                        <$output>::name(&cfg),
                    ));
                    let details = concat!($($details, "\n",)*).trim();
                    Ok(agent::OperationDescription {
                        name: $op.into(),
                        summary: $summary.trim().into(),
                        details: if details.is_empty() {
                            None
                        } else {
                            Some(details.into())
                        },
                        confirmation: operation_confirmation!($confirmation $($condition)?),
                        typescript,
                    })
                })*)*
                _ => Err(CallError::bad_input(format!("不存在操作 {name}"))),
            }
        }

        /// TypeScript declarations of every type the operations use, and the `Operations` map the
        /// UI's typed `call` is built on.
        pub fn typescript() -> String {
            let cfg = Config::new().with_large_int("number");
            let mut decls = BTreeMap::new();
            collect::<CallError>(&cfg, &mut decls);
            collect::<crawl::runner::CrawlFinished>(&cfg, &mut decls);
            collect::<matching::MatchFinished>(&cfg, &mut decls);
            collect::<task::generation::DraftsFinished>(&cfg, &mut decls);
            collect::<ExportMentors>(&cfg, &mut decls);
            collect::<backup::RestoreInfo>(&cfg, &mut decls);
            collect::<updater::UpdateStatus>(&cfg, &mut decls);
            collect::<updater::Release>(&cfg, &mut decls);
            collect::<startup::StartupFailure>(&cfg, &mut decls);
            collect::<notifications::NotificationPermission>(&cfg, &mut decls);
            $($(
                collect::<$input>(&cfg, &mut decls);
                collect::<$output>(&cfg, &mut decls);
            )*)*
            let mut out = String::from(
                "// Generated by `cargo xtask types` from crates/api. Do not edit.\n\n",
            );
            for decl in decls.values() {
                out.push_str(&format!("export {decl}\n\n"));
            }
            out.push_str(&format!(
                "export const TAG_COLORS = {:?} as const;\n\n",
                mentor::tags::COLORS,
            ));
            out.push_str(&format!(
                "export const FEEDBACK_CHARS = {};\n\n",
                cloud_api::feedback::DESCRIPTION_CHARS,
            ));
            out.push_str("export interface Operations {\n");
            $($(
                out.push_str(&format!(
                    "  {}: {{ input: {}; output: {} }};\n",
                    $op,
                    <$input>::name(&cfg),
                    <$output>::name(&cfg),
                ));
            )*)*
            out.push_str("}\n");
            out.lines().map(str::trim_end).collect::<Vec<_>>().join("\n") + "\n"
        }
    };
}

pub async fn call_agent(
    ctx: &Context,
    op: &str,
    mut input: serde_json::Value,
    waiting: impl FnOnce(&agent::Description),
    cancelled: impl std::future::Future<Output = ()>,
) -> Result<serde_json::Value, CallError> {
    let mut log = AgentCallLog { op, success: false };
    let result = match op {
        // An agent does not decide whether agents may connect.
        op if ui_only(op) => Err(CallError::invalid("这个操作只供应用界面使用".into())),
        "addLibraryMentors" => libraries::library_add_agent(ctx, input, waiting, cancelled).await,
        _ => {
            tokio::pin!(cancelled);
            let description = tokio::select! {
                biased;
                () = &mut cancelled => return Err(CallError::invalid("这个请求已经失效".into())),
                description = agent::confirmation(ctx, op, &input) => description?,
            };
            if let Some(description) = description {
                let rendered = ctx
                    .agent_access
                    .confirm(op, description.clone(), waiting, &mut cancelled)
                    .await?;
                if let Some(current) = agent::confirmation(ctx, op, &input).await?
                    && current != description
                {
                    tracing::info!(op, "agent confirmation changed before execution");
                    return Err(CallError::invalid(
                        "等待你确认期间情况已变化，请重新发起".into(),
                    ));
                }
                match op {
                    "approveTaskEmails" => {
                        let approvals = input["emails"].as_array_mut().ok_or_else(|| {
                            CallError::internal("task approvals were parsed before confirmation")
                        })?;
                        for (approval, rendered) in approvals.iter_mut().zip(rendered) {
                            approval["html"] = rendered.html.into();
                            approval["text"] = rendered.text.into();
                        }
                    }
                    "approveFirstEmail" | "approveFollowUp" | "approveReply" => {
                        let rendered = rendered.into_iter().next().ok_or_else(|| {
                            CallError::internal("the answer's rendering count was checked")
                        })?;
                        input["html"] = rendered.html.into();
                        input["text"] = rendered.text.into();
                    }
                    _ => {}
                }
            }
            call(ctx, op, input).await
        }
    };
    log.success = result.is_ok();
    if log.success {
        ctx.agent_access.emit(agent::Event::Called);
        ctx.phones.push("called-elsewhere", "null");
    }
    result
}

// Cancellation before execution must also record the call's failure.
struct AgentCallLog<'a> {
    op: &'a str,
    success: bool,
}

impl Drop for AgentCallLog<'_> {
    fn drop(&mut self) {
        tracing::info!(op = self.op, success = self.success, "agent call");
    }
}

operations! {
    "账号" {
        /// 从本机读取账号状态和邮箱，不连接服务器。
        "getAccount" [none] => get_account(Nothing) -> account::State;
        /// 读取服务器上的 Plus 到期时间。
        "getPlus" [none] => get_plus(Nothing) -> account::Plus;
        /// 读取可以买的 Plus 时长和邮件追踪额度包：价格（分）、现在买的话到期到哪天、能不能买、最多能买到哪天，以及这个账号是不是退过款。
        "getPlusItems" [none] => get_plus_items(Nothing) -> cloud_api::plus::Items;
        /// 下单购买一种 Plus 时长或额度包，返回订单编号和付款页地址；用户在浏览器里打开付款页完成付款。
        "createPlusOrder" [confirm] => create_plus_order(PlusOrderInput) -> cloud_api::plus::Created;
        /// 查订单开通了没有：pending 是还没收到付款；applied 是已开通，带账号现在的 Plus 到期时间和追踪额度；refunded 是开通后已退款或收回。
        "getPlusOrder" [none] => get_plus_order(PlusOrderId) -> cloud_api::plus::OrderState;
        /// 同意应用里这一版的用户服务协议和隐私政策。
        "acceptTerms" [confirm] => accept_terms(Nothing) -> ();
        /// 读取邀请码、已邀请人数和填写期限。
        "getInvite" [none] => get_invite(Nothing) -> cloud_api::Invite;
        /// 填写同学的邀请码，双方各得 3 天 Plus。
        "fillInvite" [none] => fill_invite(cloud_api::CodeRequest) -> account::Plus;
        /// 兑换 Plus 天数，返回天数和新的到期时间。
        "redeemCode" [none] => redeem_code(cloud_api::CodeRequest) -> cloud_api::Redeemed;
        /// 向注册邮箱发送验证码，60 秒后可重发，10 分钟内有效。
        "sendRegistrationCode" [confirm] => send_registration_code(AccountEmail) -> ();
        /// 用验证码注册并登录；密码至少 8 位，只在本机派生钥匙。
        "registerAccount" [none] => register_account(AccountVerification) -> account::State;
        /// 登录账号；密码只在本机派生钥匙。账号正在注销时返回 account_deleting，带 restore 再登录一次撤销注销。
        "loginAccount" [none] => login_account(AccountLogin) -> account::State;
        /// 向已注册的邮箱发送重置密码验证码。
        "sendPasswordResetCode" [none] => send_password_reset_code(AccountEmail) -> ();
        /// 用验证码重置密码，所有设备的旧会话失效，旧的云端备份用新密码解不开。
        "resetAccountPassword" [confirm] => reset_account_password(AccountVerification) -> ();
        /// 撤销当前设备的会话并退出登录。
        "logoutAccount" [none] => logout_account(Nothing) -> ();
        /// 输入邮箱和密码注销账号：15 天冷静期后删除服务器上的全部账号数据，期间登录可以撤销；本机数据保留。返回删除时间。
        "deleteAccount" [confirm] => delete_account(AccountCredentials) -> i64;
    }
    "云端备份" {
        /// 读取本机的云端备份设置、上次备份的时间和最近的失败，不连接服务器。
        "getCloudBackup" [none] => get_cloud_backup(Nothing) -> backup::cloud::Status;
        /// 读取 Plus 到期时间和云端那份备份：大小、是不是这台电脑的、导出时间和导师、邮件数。
        "getCloudBackupServer" [none] => get_cloud_backup_server(Nothing) -> backup::cloud::Server;
        /// 开启每天自动备份，把加密后的数据上传到官方服务器。云端是另一台电脑的备份时返回它的信息，
        /// 用 replace 带上它的 committedAt 再调用一次才替换。没有备份钥匙时要带上账号密码。
        "enableCloudBackup" [confirm] => enable_cloud_backup(backup::cloud::Enable) -> backup::cloud::Enabled;
        /// 关闭自动备份，云端已有的备份保留。
        "disableCloudBackup" [none] => disable_cloud_backup(Nothing) -> ();
        /// 立即备份到云端，数据没变时不上传。
        "backUpToCloud" [none] => back_up_to_cloud(Nothing) -> ();
        /// 删除云端的备份并关闭自动备份，本机数据不动。
        "deleteCloudBackup" [confirm] => delete_cloud_backup(Nothing) -> ();
    }
    "邮件追踪" {
        /// 读取开关、上次同步失败的原因和服务器上的 Plus 到期时间、追踪额度（读不到时带上原因）。
        "getTracking" [none] => get_tracking(Nothing) -> tracking::Status;
        /// 打开或关闭邮件追踪；打开要已登录、Plus 有效。
        "setTracking" [none] => set_tracking(tracking::TrackingSwitch) -> ();
    }
    "导师库" {
        /// 读取导师库的机构和人数。
        "getLibraryOrgs" [none] => library_orgs(Nothing) -> cloud_api::library::Orgs;
        /// 按机构、关键词和页码读取导师库，每页 50 位，带本机标记。
        "listLibraryMentors" [none] => library_mentors(cloud_api::library::ListRequest) -> library::Mentors;
        /// 预览导师库记录，不含邮箱，带本机标记。
        "previewLibraryMentor" [none] => library_preview(LibraryId) -> library::Preview;
        /// 查询今天还能添加多少位。
        "getLibraryQuota" [none] => library_quota(Nothing) -> cloud_api::library::Quota;
        /// 取得完整资料并预览添加结果；第一次取得记录会计入今天的数量。
        "previewLibraryAdd" [none] => library_add_preview(library::add::Selection) -> library::add::Preview;
        /// 按预览中的决定添加导师，再次取得记录不重复计数。
        "addLibraryMentors" [confirm_if "添加会更改已批准邮件的收件地址时"] =>
            library_add(library::add::Selection) -> library::add::Added;
        /// 按项分组读取导师库的待处理资料更新，take 为默认选择。
        "listLibraryUpdates" [none] => library_updates(Nothing) -> Vec<library::updates::Group>;
        /// 应用逐行选择并清除处理过的更新，返回修改的导师数。
        "applyLibraryUpdates" [confirm_if "更新会更改已批准邮件的收件地址时"] =>
            library_apply(library::updates::Choices) -> u32;
        /// 查询有待处理资料更新的导师数，不含回收站。
        "libraryUpdateCount" [none] => library_update_count(Nothing) -> i64;
        /// 导师库同步从什么时候起一直失败，以及最近一次的原因；没有失败时为 null。
        "libraryFailure" [none] => library_failure(Nothing) -> Option<libraries::LibraryFailure>;
        /// 立即从同步进度检查导师库更新，返回处理的变化数。
        "syncLibrary" [none] => library_sync(Nothing) -> u32;
    }
    "Agent 连接" {
        /// 读取命令行工具的安装状态、命令名和绝对路径。
        "getAgentCli" [none] => get_agent_cli(Nothing) -> AgentCli;
        /// 将 autoemail 安装到终端 PATH。
        "installAgentCli" [none] => install_agent_cli(Nothing) -> AgentCli;
        /// 撤回应用添加的命令行链接或用户 PATH 项。
        "uninstallAgentCli" [none] => uninstall_agent_cli(Nothing) -> AgentCli;
        /// 预览指定客户端的 MCP 配置路径、完整服务项及是否替换已有项。
        "previewMcpConfig" [none] => preview_mcp_config(McpConfigQuery) -> McpConfigPreview;
        /// 确认后写入指定客户端最近预览的配置，原文件变化时需重新预览。
        "writeMcpConfig" [none] => write_mcp_config(McpConfigQuery) -> ();
        /// 读取供其他 MCP 工具复制的 JSON 配置。
        "mcpConfigText" [none] => mcp_config_text(Nothing) -> String;
        /// 读取是否允许 Agent 连接。
        "getAgentAccess" [none] => get_agent_access(Nothing) -> bool;
        /// 设置是否允许 Agent 连接，enabled 为是否开启。
        "setAgentAccess" [none] => set_agent_access(agent::AgentAccess) -> ();
        /// 列出等待用户回答的 Agent 请求。
        "listAgentRequests" [none] => list_agent_requests(Nothing) -> Vec<agent::AgentRequest>;
        /// 回答指定 id 的 Agent 请求，allow 为是否允许。
        "answerAgentRequest" [none] => answer_agent_request(agent::AgentAnswer) -> ();
        /// 按组列出 Agent 可调用的操作、说明和确认方式。
        "listOperations" [none] => list_operations(Nothing) -> Vec<agent::OperationSummary>;
        /// 按 name 读取操作说明、确认方式及输入输出的 TypeScript 类型声明。
        "describeOperation" [none] => describe_operation(agent::OperationName)
            -> agent::OperationDescription;
    }
    "手机远程" {
        /// 读取是否允许手机连接，以及和中继的连接状态。
        "getPhoneAccess" [none] => get_phone_access(Nothing) -> PhoneAccess;
        /// 设置是否允许手机连接，关掉时断开所有手机。
        "setPhoneAccess" [none] => set_phone_access(PhoneAccessEdit) -> ();
        /// 生成配对用的二维码，5 分钟内有效、只能用一次，替换之前的。
        "addPhone" [none] => add_phone(Nothing) -> PhoneCode;
        /// 回答扫码的手机能不能配对，allow 为是否允许。
        "answerPhonePairing" [none] => answer_phone_pairing(PhonePairingAnswer) -> ();
        /// 作废正在显示的二维码。
        "cancelPhonePairing" [none] => cancel_phone_pairing(Nothing) -> ();
        /// 列出已配对的手机和它们是否在线。
        "listPhones" [none] => list_phones(Nothing) -> Vec<PairedPhone>;
        /// 撤销一部手机的配对并断开它。
        "revokePhone" [none] => revoke_phone(PhoneId) -> ();
    }
    "概览" {
        /// 读取后台保存失败的提示，没有失败时返回 null。
        "storageFailure" [none] => storage_failure(Nothing) -> Option<String>;
        /// 按筛选条件统计联系导师的进展。
        "contactStats" [none] => contact_stats(StatsQuery) -> stats::ContactProgress;
        /// 统计导师库概况。
        "mentorStats" [none] => mentor_stats(Nothing) -> stats::MentorOverview;
        /// 统计指定时段的模型用量。
        "tokenStats" [none] => token_stats(StatsPeriod) -> stats::TokenUsage;
        /// 列出指定时段内每次调用模型的记录，最近的在前，可按用途和模型筛选。
        "listModelCalls" [none] => list_model_calls(ModelCallQuery) -> stats::ModelCallPage;
        /// 查询今天待处理的邮件和任务。
        "todayWork" [none] => today_work(TodayWorkQuery) -> stats::TodayWork;
    }
    "整理与候选" {
        /// 把你从学校官网整理的导师送进候选区，由用户审核后入库；收集导师前先读这个操作的说明。
        #[doc = include_str!("submit-candidates.md")]
        "submitCandidates" [none] => submit_candidates(crawl::submit::SubmitCandidates)
            -> crawl::submit::Submitted;
        /// 检查并规范化整理用的网址，urls 为按行分隔的网址。
        "checkCrawlUrls" [none] => check_crawl_urls(CrawlUrls) -> Vec<String>;
        /// 查询指定时间范围内整理的后台进度。
        "crawlWork" [none] => crawl_work(CrawlWorkQuery) -> Vec<crawl::queue::CrawlSummary>;
        /// 从给定网址开始整理导师信息。
        "startCrawl" [none] => start_crawl(CrawlStart) -> CrawlId;
        /// 分页列出整理记录。
        "listCrawls" [none] => list_crawls(CrawlQuery) -> crawl::queue::CrawlList;
        /// 列出可用于筛选的整理记录。
        "crawlOptions" [none] => crawl_options(Nothing) -> Vec<crawl::queue::CrawlOption>;
        /// 读取指定 id 的整理详情。
        "getCrawl" [none] => get_crawl(CrawlDetailQuery) -> crawl::queue::CrawlDetail;
        /// 这次整理用掉的 Token，和按模型现在填的单价估算的花费。
        "crawlUsage" [none] => crawl_usage(CrawlId) -> stats::TokenTotals;
        /// 修改智能整理认出的学校和学院，school 为空表示全校；这次整理待审核的候选在原学校学院的单位跟着改。
        /// Agent 提交和补全已有导师的记录不能改。
        "setCrawlUnit" [none] => set_crawl_unit(CrawlUnit) -> ();
        /// 暂停指定整理。
        "pauseCrawl" [none] => pause_crawl(CrawlId) -> ();
        /// 继续指定整理。
        "resumeCrawl" [none] => resume_crawl(CrawlId) -> ();
        /// 取消指定整理。
        "cancelCrawl" [none] => cancel_crawl(CrawlId) -> ();
        /// 重试整理中失败或指定的页面。
        "retryCrawl" [none] => retry_crawl(CrawlRetry) -> u64;
        /// 永久删除指定整理及尚未审核的候选资料。
        "deleteCrawl" [confirm] => delete_crawl(CrawlId) -> ();
        /// 预览指定候选导师可补充的资料。
        "previewEnrich" [none] => preview_enrich(CandidateIds) -> crawl::enrich::EnrichPlan;
        /// 为指定候选导师补充资料并返回处理计划。
        "enrichCandidates" [none] => enrich_candidates(CandidateIds) -> crawl::enrich::EnrichPlan;
        /// 预览给指定导师补全资料：补全几位、跳过哪些和原因。
        "previewEnrichMentors" [none] => preview_enrich_mentors(MentorIds) -> crawl::enrich::EnrichPlan;
        /// 按指定导师现有的资料生成候选并补全，补到的在候选里审核后才并进导师。
        "enrichMentors" [none] => enrich_mentors(MentorIds) -> crawl::enrich::MentorEnrichment;
        /// 按筛选条件分页列出候选导师。
        "listCandidates" [none] => list_candidates(crawl::review::CandidateQuery)
            -> crawl::review::CandidatePage;
        /// 按筛选条件列出全部候选导师的 id，用于全选。
        "listCandidateIds" [none] => list_candidate_ids(crawl::review::CandidateSelection)
            -> Vec<i64>;
        /// 读取指定 id 的候选导师详情。
        "getCandidate" [none] => get_candidate(CrawlId) -> crawl::review::CandidateDetail;
        /// 预览候选导师入库，包含可能重复的导师。
        "previewCandidates" [none] => preview_candidates(crawl::review::ReviewInput)
            -> mentor::intake::Preview;
        /// 按审核决定将候选导师保存到导师库。
        "acceptCandidates" [confirm_if "候选入库会更改已批准邮件的收件地址时"] =>
            accept_candidates(crawl::review::ReviewInput) -> crawl::review::Reviewed;
        /// 拒绝指定 id 列表中的候选导师。
        "rejectCandidates" [none] => reject_candidates(CandidateIds) -> u64;
    }
    "发送计划" {
        /// 估算给定发送窗口的容量。
        "scheduleCapacity" [none] => schedule_capacity(email::schedule::Window)
            -> email::schedule::Capacity;
        /// 预览任务按给定时间安排邮件的结果。
        "previewTaskSchedule" [none] => preview_task_schedule(task::ApprovalPreview)
            -> task::SchedulePreview;
        /// 更改指定任务的发送时间安排。
        "changeTaskSchedule" [confirm_if "任务里有已批准的邮件时"] => change_task_schedule(task::ScheduleEdit) -> u64;
        /// 更改指定邮件的发送时间，plannedAt 为 UTC 毫秒时间戳。
        "rescheduleEmail" [confirm] => reschedule_email(RescheduleEmail) -> ();
        /// 立即发送指定 id 的已批准邮件。
        "sendEmailNow" [confirm] => send_email_now(EmailId) -> ();
        /// 按筛选条件分页读取发送计划。
        "sendingPlan" [none] => sending_plan(send::plan::PlanQuery) -> send::plan::PlanPage;
        /// 读取已排期邮件的后台进度。
        "scheduledWork" [none] => scheduled_work(Nothing) -> send::plan::ScheduledWork;
        /// 列出日历上标出的中国节假日：各个假期放假的起止日期和调休上班的日子，按北京时间的日期，
        /// 格式 YYYY-MM-DD。
        "listHolidays" [none] => list_holidays(Nothing) -> &'static holidays::Holidays;
        /// 预览交给云端发送：哪些定时的首封能交、各自排好的发件邮箱和时间，不能交的和原因，
        /// 要用到的邮箱和其中哪些要先同意交出授权码，以及大约要上传多少。
        /// emails 不填时预览全部还没发出的已批准定时邮件，填了 task 只看这个任务的；
        /// 改了勾选后用勾选的再预览一次。
        "cloudSendPreview" [none] => cloud_send_preview(cloud_send::PreviewInput)
            -> cloud_send::Preview;
        /// 把邮件交给官方服务器，到排好的时间从排好的邮箱发出，电脑关机也照发。
        /// emails 照用同一批邮件预览的结果填；consents 填用户同意交出授权码的邮箱 id。
        /// 会把这些邮件和用到的邮箱的授权码上传到官方服务器。
        "handOffToCloud" [confirm] => hand_off_to_cloud(cloud_send::HandOff)
            -> cloud_send::HandedOff;
        /// 把交给云端、还没发出的邮件撤回到本地，变回已批准；云端已经发出的带回结果。
        /// 填 email 撤回一封，填 task 撤回这个任务里的全部；返回正在发送、撤不回的封数。
        "withdrawFromCloud" [none] => withdraw_from_cloud(cloud_send::Withdraw)
            -> cloud_send::Withdrawn;
    }
    "任务" {
        /// 预览新任务的导师范围、草稿数量和模型用量。
        "planTask" [none] => plan_task(task::NewTask) -> task::TaskPlan;
        /// 创建联系任务并安排生成草稿。
        "createTask" [none] => create_task(task::NewTask) -> TaskId;
        /// 读取各任务的草稿生成进度。
        "taskWork" [none] => task_work(Nothing) -> Vec<task::TaskWork>;
        /// 统计各任务里待审核的草稿封数。
        "reviewCount" [none] => review_count(Nothing) -> i64;
        /// 按筛选条件分页列出任务。
        "listTasks" [none] => list_tasks(task::TaskQuery) -> task::TaskPage;
        /// 读取指定 id 的任务概况。
        "getTask" [none] => get_task(TaskId) -> task::TaskSummary;
        /// 按任务和筛选条件分页列出草稿。
        /// 会在后台查询收件地址的 DNS 记录，以检查是否有邮件服务器。
        "listTaskDrafts" [none] => list_task_drafts(task::DraftQuery) -> task::DraftPage;
        /// 读取任务中指定邮件的草稿详情。
        "getTaskDraft" [none] => get_task_draft(task::TaskEmail) -> task::TaskDraft;
        /// 向任务草稿添加指定材料，返回是否添加成功。
        "attachTaskMaterial" [none] => attach_task_material(TaskMaterial) -> bool;
        /// 保存任务中指定邮件的草稿。
        "saveTaskDraft" [none] => save_task_draft(task::DraftEdit) -> ();
        /// 检查给定任务草稿，返回需要处理的问题。
        "checkTaskDraft" [none] => check_task_draft(task::DraftEdit) -> Vec<check::Problem>;
        /// 根据给定任务草稿使用模型生成邮件。
        "writeTaskDraft" [none] => write_task_draft(Writing<task::DraftEdit>) -> writing::Letter;
        /// 检查并列出任务中可批准的草稿。
        /// 会在后台查询收件地址的 DNS 记录，以检查是否有邮件服务器。
        "prepareTaskApproval" [none] => prepare_task_approval(TaskId)
            -> Vec<task::ApprovalCandidate>;
        /// 批准选中的任务邮件，按各自发送时间发出。
        "approveTaskEmails" [confirm] => approve_task_emails(task::TaskApprovals) -> u64;
        /// 暂停指定任务：不再生成草稿，已批准的邮件也先不发，仍可以审核和批准。
        "pauseTask" [none] => pause_task(TaskId) -> ();
        /// 恢复指定任务：接着生成排队的草稿（会调用模型、产生用量），并发出已批准的邮件。
        "resumeTask" [confirm_if "恢复后会发出已批准的邮件时"] => resume_task(TaskId) -> ();
        /// 停止指定任务：没发出的邮件全部取消（记录保留），任务不能再继续。
        "stopTask" [none] => stop_task(TaskId) -> ();
        /// 永久删除指定任务和其中没发出的邮件（包括已批准的），发出过的留在导师的往来里。
        "deleteTask" [confirm] => delete_task(TaskId) -> ();
        /// 从任务中移除指定邮件。
        "removeTaskEmail" [none] => remove_task_email(task::TaskEmail) -> ();
        /// 安排重新生成任务中的指定草稿。
        "regenerateTaskEmail" [none] => regenerate_task_email(task::TaskEmail) -> u64;
        /// 为任务中的指定草稿重新套用模板。
        "reapplyTaskTemplate" [none] => reapply_task_template(task::TaskEmail) -> ();
        /// 重试任务中失败或未完成的草稿生成。
        "retryTaskGeneration" [none] => retry_task_generation(TaskRetry) -> u64;
        /// 读取指定任务的配置，用于再次创建任务。
        "restartTask" [none] => restart_task(TaskId) -> task::NewTask;
    }
    "收信与通知" {
        /// 按筛选条件分页列出导师会话。
        "listConversations" [none] => list_conversations(mentor::conversations::ConversationQuery)
            -> mentor::conversations::ConversationPage;
        /// 读取未读回信数量。
        "unreadCount" [none] => unread_count(Nothing) -> i64;
        /// 立即检查所有邮箱的新回信。
        "receiveNow" [none] => receive_now(Nothing) -> ReceiveStatus;
        /// 读取最近一次收信时间。
        "receiveStatus" [none] => receive_status(Nothing) -> ReceiveStatus;
        /// 读取后台工作通知开关。
        "getBackgroundNotifications" [none] => get_background_notifications(Nothing) -> bool;
        /// 设置后台工作通知，on 为是否开启。
        "setBackgroundNotifications" [none] => set_background_notifications(NotificationSetting)
            -> ();
        /// 读取 AI 邮件事实核对开关。
        "getAiFactCheck" [none] => get_ai_fact_check(Nothing) -> bool;
        /// 设置 AI 邮件事实核对，on 为是否开启。
        "setAiFactCheck" [none] => set_ai_fact_check(NotificationSetting) -> ();
        /// 用模型核对指定的待审核邮件并保存结果，核对开关关闭时返回 null。
        /// 会将邮件、个人资料和参考材料发给已同意的模型服务，并产生用量；读取已有结果请用草稿详情。
        "factCheckEmail" [none] => fact_check_email(EmailId)
            -> Option<email::fact_check::FactCheck>;
        /// 读取新回信通知开关。
        "getReplyNotifications" [none] => get_reply_notifications(Nothing) -> bool;
        /// 设置新回信通知，on 为是否开启。
        "setReplyNotifications" [none] => set_reply_notifications(NotificationSetting) -> ();
    }
    "匹配" {
        /// 估算指定导师的匹配数量和模型用量，redo 表示是否重算已有结果。
        "planMatching" [none] => plan_matching(MatchBatch) -> matching::MatchEstimate;
        /// 安排指定导师进行匹配，redo 表示是否重算已有结果。
        "startMatching" [none] => start_matching(MatchBatch) -> u64;
        /// 为指定 id 的导师重新计算匹配度。
        "matchMentor" [none] => match_mentor(MentorId) -> u64;
        /// 读取匹配队列进度。
        "matchingProgress" [none] => matching_progress(Nothing) -> mentor::matching::MatchProgress;
        /// 暂停导师匹配。
        "pauseMatching" [none] => pause_matching(Nothing) -> ();
        /// 继续导师匹配。
        "resumeMatching" [none] => resume_matching(Nothing) -> ();
        /// 取消尚未完成的导师匹配。
        "cancelMatching" [none] => cancel_matching(Nothing) -> ();
        /// 重试失败的导师匹配。
        "retryMatching" [none] => retry_matching(Nothing) -> u64;
    }
    "导师" {
        /// 预览表格导入，包含字段映射和重复导师处理结果。
        "previewImport" [none] => preview_import(PreviewImport) -> mentor::import::Preview;
        /// 按字段映射和重复处理决定导入导师表格。
        "importMentors" [confirm_if "导入会更改已批准邮件的收件地址时"] =>
            import_mentors(ImportMentors) -> mentor::import::Imported;
        /// 按筛选、排序和分页条件读取导师列表。
        "listMentors" [none] => list_mentors(mentor::MentorQuery) -> mentor::MentorPage;
        /// 按筛选条件读取筛出的全部导师的 id，用来一次勾选它们。
        "listMentorIds" [none] => list_mentor_ids(mentor::filters::MentorFilter) -> Vec<i64>;
        /// 读取导师筛选里哪些选项能用：有没有追踪过打开情况的信。
        "mentorFilterOptions" [none] => mentor_filter_options(Nothing)
            -> mentor::filters::MentorFilterOptions;
        /// 读取一项单位筛选（学校、学院、系所或职称）可选的值：按其余筛选条件筛出的导师里，
        /// 各值有几位，没填这一项的有几位。职称按拆开的每一项算。
        "mentorFilterValues" [none] => mentor_filter_values(FilterValuesQuery)
            -> mentor::filters::FilterValues;
        /// 读取标签筛选可选的标签：按其余筛选条件筛出的导师里，各标签有几位，没有标签的有几位。
        "mentorFilterTags" [none] => mentor_filter_tags(mentor::filters::MentorFilter)
            -> mentor::filters::FilterTags;
        /// 读取指定 id 的导师完整资料。
        "getMentor" [none] => get_mentor(MentorId) -> mentor::MentorDetail;
        /// 保存指定导师的备注，返回存下的备注；只有空白时清空备注，返回 null。
        "setMentorNote" [none] => set_mentor_note(MentorNote) -> Option<String>;
        /// 添加一位导师，返回新导师的 id。
        "addMentor" [none] => add_mentor(mentor::MentorForm) -> MentorId;
        /// 更新指定导师的资料。before 是编辑前 getMentor 读到的 name、aliases、emails、
        /// affiliations、homepages、research；这之后资料有了变化就不保存，重新读取后再改。
        "updateMentor" [confirm_if "更改已批准邮件的收件地址时"] =>
            update_mentor(MentorEdit) -> mentor::MentorDetail;
        /// 设置联系导师使用的邮箱，mailboxId 为 null 时取消指定。
        "setMentorMailbox" [confirm_if "这位导师有已批准尚未发出的邮件，且发件邮箱改变时"] =>
            set_mentor_mailbox(MentorMailbox) -> mentor::MentorDetail;
        /// 为指定导师添加邮箱地址，address 为新地址。
        "addMentorEmail" [confirm_if "新地址成为已批准邮件的收件地址时"] =>
            add_mentor_email(MentorAddress) -> mentor::MentorDetail;
        /// 预览把 other 导师并入 keep 导师的结果。
        "previewMerge" [none] => preview_merge(MentorMerge) -> mentor::merge::Preview;
        /// 把 other 导师合并到 keep 导师。discardedDrafts 是 previewMerge 给出的；这之后要删的草稿
        /// 有了变化就不合并，重新预览后再合并。
        "mergeMentors" [confirm] => merge_mentors(PreviewedMerge) -> ();
        /// 按筛选和分页条件读取回收站导师。
        "listTrash" [none] => list_trash(mentor::trash::TrashQuery) -> mentor::MentorPage;
        /// 将指定 id 列表中的导师移入回收站。
        "trashMentors" [none] => trash_mentors(MentorIds) -> ();
        /// 从回收站恢复指定 id 列表中的导师。
        "restoreMentors" [none] => restore_mentors(MentorIds) -> ();
        /// 永久删除指定 id 列表中的导师。
        "deleteMentors" [confirm] => delete_mentors(MentorIds) -> ();
    }
    "标签" {
        /// 为指定导师添加或移除标签。
        "tagMentors" [none] => tag_mentors(TagMentors) -> ();
        /// 列出全部导师标签。
        "listTags" [none] => list_tags(Nothing) -> Vec<mentor::tags::Tag>;
        /// 创建标签，并可同时添加到 mentorIds 指定的导师。
        "createTag" [none] => create_tag(NewTag) -> mentor::tags::Tag;
        /// 更改指定标签的名称。
        "renameTag" [none] => rename_tag(TagName) -> ();
        /// 更改指定标签的颜色。
        "recolorTag" [none] => recolor_tag(TagColor) -> ();
        /// 调整指定标签的显示顺序。
        "moveTag" [none] => move_tag(TagMove) -> ();
        /// 永久删除指定标签。
        "deleteTag" [confirm] => delete_tag(TagId) -> ();
    }
    "邮箱" {
        /// 列出支持的邮箱服务商及连接设置。
        "listProviders" [none] => list_providers(Nothing)
            -> &'static [mailbox::providers::Provider];
        /// 列出已配置的邮箱，不包含授权码。
        "listMailboxes" [none] => list_mailboxes(Nothing) -> Vec<mailbox::Mailbox>;
        /// 读取多邮箱轮换设置。
        "getRotation" [none] => get_rotation(Nothing) -> profile::Rotation;
        /// 保存多邮箱轮换设置。
        "setRotation" [none] => set_rotation(profile::Rotation) -> ();
        /// 添加发信和收信邮箱，返回新邮箱的 id。
        "addMailbox" [none] => add_mailbox(NewMailbox) -> MailboxId;
        /// 修改指定邮箱设置，可填写新的授权码。
        "updateMailbox" [confirm_if "修改未到期暂停中的邮箱设置会恢复发出已批准的邮件，或更改邮箱地址会影响已批准邮件时"] =>
            update_mailbox(MailboxEdit) -> ();
        /// 立即恢复指定邮箱的发信。
        "resumeMailbox" [confirm_if "恢复后会发出已批准的邮件时"] => resume_mailbox(MailboxId) -> ();
        /// 永久删除指定邮箱。
        "deleteMailbox" [confirm] => delete_mailbox(MailboxId) -> ();
        /// 测试给定邮箱配置能否连接发信服务器。
        "testSending" [none] => test_sending(MailboxTest) -> ();
        /// 测试给定邮箱配置能否连接收信服务器。
        "testReceiving" [none] => test_receiving(MailboxTest) -> ();
        /// 通过指定邮箱向它自己的地址发送测试信。
        "sendTestMail" [none] => send_test_mail(MailboxId) -> ();
    }
    "个人资料" {
        /// 读取学生个人资料。
        "getProfile" [none] => get_profile(Nothing) -> profile::Profile;
        /// 保存学生个人资料，只改传了的字段，空字符串清空。
        "updateProfile" [none] => update_profile(profile::ProfileForm) -> profile::Profile;
        /// 标记首次使用引导已完成。
        "finishOnboarding" [none] => finish_onboarding(Nothing) -> ();
        /// 读取首次使用引导中的测试邮件草稿。
        "testLetter" [none] => test_letter(Nothing) -> Option<TestLetter>;
        /// 向用户自己的邮箱发送引导测试邮件。
        "sendTestLetter" [none] => send_test_letter(SendTestLetter) -> ();
        /// 读取邮件写作风格设置。
        "getLetterStyle" [none] => get_letter_style(Nothing) -> profile::LetterStyle;
        /// 保存邮件写作风格设置。
        "updateLetterStyle" [none] => update_letter_style(profile::LetterStyle) -> ();
    }
    "材料" {
        /// 列出个人材料及其用途。
        "listMaterials" [none] => list_materials(Nothing) -> Vec<material::Material>;
        /// 保存 Base64 编码的材料文件并提取文字，返回材料 id。
        "addMaterial" [none] => add_material(UploadedFile) -> MaterialId;
        /// 设置指定材料用于匹配、写信或作为附件。
        "setMaterialPurpose" [none] => set_material_purpose(MaterialPurpose) -> ();
        /// 读取指定材料提取出的文字。
        "getMaterialText" [none] => get_material_text(MaterialId) -> Option<String>;
        /// 永久删除指定材料。
        "deleteMaterial" [confirm] => delete_material(MaterialId) -> ();
    }
    "模板" {
        /// 列出邮件模板支持的占位符。
        "listPlaceholders" [none] => list_placeholders(Nothing) -> &'static [&'static str];
        /// 列出邮件模板摘要。
        "listTemplates" [none] => list_templates(Nothing) -> Vec<template::TemplateSummary>;
        /// 读取指定 id 的完整邮件模板。
        "getTemplate" [none] => get_template(TemplateId) -> template::Template;
        /// 解析 Base64 编码的 Word 文件为邮件模板内容。
        "importWordTemplate" [none] => import_word_template(UploadedFile) -> WordFile;
        /// 创建邮件模板，返回模板 id。
        "addTemplate" [none] => add_template(template::TemplateForm) -> TemplateId;
        /// 更新指定邮件模板。
        "updateTemplate" [none] => update_template(TemplateEdit) -> ();
        /// 永久删除指定邮件模板。
        "deleteTemplate" [confirm] => delete_template(TemplateId) -> ();
        /// 设置默认邮件模板，id 为 null 时取消默认。
        "setDefaultTemplate" [none] => set_default_template(DefaultTemplate) -> ();
        /// 将指定模板填入指定导师的资料。
        "applyTemplate" [none] => apply_template(TemplateForMentor) -> FilledTemplate;
        /// 为指定导师填入默认模板，没有默认模板时返回 null。
        "applyDefaultTemplate" [none] => apply_default_template(MentorId) -> Option<FilledTemplate>;
    }
    "追问" {
        /// 读取导师追问入口及可用状态。
        "getFollowUpEntry" [none] => get_follow_up_entry(MentorId)
            -> email::follow_up::FollowUpEntry;
        /// 读取指定导师已有的追问草稿。
        "getFollowUpDraft" [none] => get_follow_up_draft(MentorId)
            -> Option<email::follow_up::FollowUpDraft>;
        /// 为指定导师创建追问草稿。
        "createFollowUpDraft" [none] => create_follow_up_draft(MentorId)
            -> email::follow_up::FollowUpDraft;
        /// 保存指定导师的追问草稿。
        "saveFollowUpDraft" [none] => save_follow_up_draft(FollowUpEdit)
            -> Option<email::follow_up::FollowUpDraft>;
        /// 丢弃指定 id 的追问草稿。
        "discardFollowUpDraft" [none] => discard_follow_up_draft(EmailId) -> ();
        /// 批准指定导师的追问草稿并安排发送。
        "approveFollowUp" [confirm] => approve_follow_up(ReplyEmail) -> ();
        /// 根据给定草稿使用模型撰写追问邮件。
        "writeFollowUp" [none] => write_follow_up(Writing<FirstDraftEdit>) -> writing::Letter;
        /// 检查给定追问草稿，返回需要处理的问题。
        "checkFollowUpDraft" [none] => check_follow_up_draft(ReplyDraftEdit) -> Vec<check::Problem>;
    }
    "回复" {
        /// 读取指定导师的回复草稿。
        "getReplyDraft" [none] => get_reply_draft(MentorId) -> Option<email::ReplyDraft>;
        /// 读取导师可回复的目标邮件，支持指定回信。
        "getReplyTarget" [none] => get_reply_target(ReplyTargetQuery) -> Option<email::ReplyTarget>;
        /// 保存导师回复草稿，空内容且无附件时删除已有草稿。
        "saveReplyDraft" [confirm_if "空保存会删除已有回复草稿时"] =>
            save_reply_draft(ReplyDraftEdit) -> Option<email::ReplyDraft>;
        /// 检查给定回复草稿，返回需要处理的问题。
        "checkReplyDraft" [none] => check_reply_draft(ReplyDraftEdit) -> Vec<check::Problem>;
        /// 批准指定导师的回复草稿并安排发送。
        "approveReply" [confirm] => approve_reply(ReplyEmail) -> ();
        /// 根据给定草稿使用模型撰写回复邮件。
        "writeReply" [none] => write_reply(Writing<ReplyDraftEdit>) -> writing::Letter;
        /// 向指定导师的邮件草稿添加材料。
        "attachDraftMaterial" [none] => attach_draft_material(DraftMaterial) -> bool;
    }
    "首封" {
        /// 读取指定导师独立于任务的首封草稿。
        "getFirstDraft" [none] => get_first_draft(MentorId) -> Option<email::FirstDraft>;
        /// 保存导师首封草稿，主题和正文均为空时删除已有草稿。
        "saveFirstDraft" [confirm_if "空保存会删除已有首封草稿时"] =>
            save_first_draft(FirstDraftEdit) -> Option<email::FirstDraft>;
        /// 检查给定首封草稿，返回需要处理的问题。
        "checkFirstDraft" [none] => check_first_draft(FirstDraftEdit) -> Vec<check::Problem>;
        /// 根据给定草稿使用模型撰写首封邮件。
        "writeLetter" [none] => write_letter(Writing<FirstDraftEdit>) -> writing::Letter;
        /// 撤回还在排队等模型的 AI 写信请求；已经发给模型的照常完成。
        "cancelWriting" [none] => cancel_writing(WritingRequest) -> ();
        /// 查询邮箱域名的 DNS 记录，检查是否有邮件服务器，返回需要处理的问题。
        "checkMailDomain" [none] => check_mail_domain(MailAddress) -> Option<check::Problem>;
        /// 批准指定导师的首封草稿并安排发送。
        "approveFirstEmail" [confirm] => approve_first_email(FirstEmail) -> ();
    }
    "待办与回信处理" {
        /// 读取 AI 从回信提取待办的开关。
        "getAiReplyTodos" [none] => get_ai_reply_todos(Nothing) -> bool;
        /// 设置 AI 从回信提取待办的开关。
        "setAiReplyTodos" [none] => set_ai_reply_todos(AiReplyTodos) -> ();
        /// 读取待办提醒开关。
        "getTodoReminders" [none] => get_todo_reminders(Nothing) -> bool;
        /// 设置待办提醒，on 为是否开启。
        "setTodoReminders" [none] => set_todo_reminders(NotificationSetting) -> ();
        /// 为指定导师添加待办，返回待办 id。
        "addTodo" [none] => add_todo(NewTodo) -> i64;
        /// 更新指定待办的内容和时间。
        "updateTodo" [none] => update_todo(TodoEdit) -> ();
        /// 设置指定待办是否完成。
        "setTodoDone" [none] => set_todo_done(TodoDone) -> ();
        /// 永久删除指定待办。
        "deleteTodo" [confirm] => delete_todo(TodoId) -> ();
        /// 按筛选和分页条件读取待办。
        "listTodos" [none] => list_todos(email::todos::TodoQuery) -> email::todos::TodoPage;
        /// 安排使用模型重新从指定回信提取待办。
        "retryReplyTodos" [none] => retry_reply_todos(ReplyMessage) -> ();
        /// 读取 AI 识别回信结果的开关。
        "getAiReplyOutcome" [none] => get_ai_reply_outcome(Nothing) -> bool;
        /// 设置 AI 识别回信结果的开关。
        "setAiReplyOutcome" [none] => set_ai_reply_outcome(AiReplyOutcome) -> ();
        /// 设置指定回信的联系结果。
        "setReplyOutcome" [none] => set_reply_outcome(ReplyOutcomeEdit) -> ();
        /// 安排使用模型重新识别指定回信的联系结果。
        "retryReplyOutcome" [none] => retry_reply_outcome(ReplyMessage) -> ();
        /// 读取 AI 回信处理队列状态。
        "replyProcessing" [none] => reply_processing(Nothing) -> email::outcome::ReplyProcessing;
        /// 继续已暂停的 AI 回信处理，会调用已同意的模型服务。
        "resumeReplyProcessing" [none] => resume_reply_processing(Nothing) -> ();
    }
    "往来邮件" {
        /// 读取指定导师的往来邮件和状态记录。
        "listCorrespondence" [none] => list_correspondence(MentorId) -> Vec<Entry>;
        /// 将指定的回信标为已读。
        "markRead" [none] => mark_read(MessageIds) -> ();
        /// 将尚未发出的邮件撤回为草稿。
        "withdrawEmail" [none] => withdraw_email(EmailId) -> ();
        /// 重试发送指定 id 的失败邮件。
        "retryEmail" [confirm] => retry_email(EmailId) -> ();
        /// 确认指定邮件没有发出并结束待确认的取消状态。
        "confirmUnsentEmail" [none] => confirm_unsent_email(EmailId) -> ();
        /// 按原内容重新发送指定 id 的邮件。
        "resendEmail" [confirm] => resend_email(EmailId) -> ();
    }
    "模型" {
        /// 列出支持的模型服务商。
        "listModelProviders" [none] => list_model_providers(Nothing)
            -> &'static [model::providers::Provider];
        /// 列出模型配置，不包含 API Key。
        "listModelConfigs" [none] => list_model_configs(Nothing) -> Vec<model::ModelConfig>;
        /// 添加模型配置，返回配置 id。
        "addModelConfig" [none] => add_model_config(NewModelConfig) -> ModelConfigId;
        /// 更新指定模型配置，可填写新的 API Key。
        "updateModelConfig" [none] => update_model_config(ModelConfigEdit) -> ();
        /// 永久删除指定模型配置。
        "deleteModelConfig" [confirm] => delete_model_config(ModelConfigId) -> ();
        /// 设置默认模型配置。
        "setDefaultModel" [none] => set_default_model(ModelConfigId) -> ();
        /// 给一项功能单独选模型配置；id 为空时跟随默认模型。
        /// 功能：crawl 智能整理和信息补全，match 匹配分析，writing AI 写信，check AI 核对，reply 回信处理。
        "setFeatureModel" [none] => set_feature_model(FeatureModel) -> ();
        /// 同意将数据发送到指定模型服务商和地址。
        /// 这会允许应用在使用该模型时发送导师、邮件或材料等相关数据。
        "agreeModel" [confirm] => agree_model(ModelAgreement) -> ();
        /// 向给定模型服务请求可用模型名称。
        "listProviderModels" [none] => list_provider_models(ModelSource) -> Vec<model::ListedModel>;
        /// 向给定模型服务发送测试请求，检查模型能否响应，可能产生用量。
        "testModel" [none] => test_model(ModelTest) -> ();
    }
    "界面" {
        /// 把界面自身的错误记进日志。
        "logUiError" [none] => log_ui_error(UiErrorReport) -> ();
    }
    "问题反馈" {
        /// 打开反馈弹窗时读取要附上的日志：大小和起止日期。上次提交没确认成功的，沿用那次的日志。
        "previewFeedback" [none] => preview_feedback(Nothing) -> FeedbackPreview;
        /// 读取预览的那份日志的全文，已打码，就是要发出的内容。
        "getFeedbackLog" [none] => get_feedback_log(Nothing) -> String;
        /// 把问题说明和可选的日志经官方服务器发到客服邮箱；账号、说明和是否附日志都和上次没确认成功的相同时算重试。
        "submitFeedback" [none] => submit_feedback(FeedbackInput) -> ();
    }
}

/// The input of operations that need none.
#[derive(Debug, Deserialize, TS)]
pub struct Nothing {}

/// Adds the declarations of `T` and every named type it refers to, keyed by name.
fn collect<T: TS + 'static + ?Sized>(cfg: &Config, decls: &mut BTreeMap<String, String>) {
    struct Visitor<'a> {
        cfg: &'a Config,
        decls: &'a mut BTreeMap<String, String>,
    }
    impl TypeVisitor for Visitor<'_> {
        fn visit<T: TS + 'static + ?Sized>(&mut self) {
            // Only named types have a path to export to; primitives and containers are inlined.
            if T::output_path().is_none() {
                return;
            }
            // A generic type has one declaration for all its uses.
            let (name, decl) = (T::ident(self.cfg), T::decl(self.cfg));
            match self.decls.get(&name) {
                Some(seen) => assert_eq!(seen, &decl, "two types are named {name}"),
                None => {
                    self.decls.insert(name, decl);
                    T::visit_dependencies(self);
                }
            }
            T::visit_generics(self);
        }
    }
    let mut visitor = Visitor { cfg, decls };
    visitor.visit::<T>();
    // What a container such as `Vec<T>` holds.
    T::visit_generics(&mut visitor);
}

/// An error for the log, with the outside words it quotes.
fn diagnosed(e: &impl std::fmt::Display, diagnostic: Option<String>) -> String {
    match diagnostic {
        Some(words) => format!("{e} ({words})"),
        None => e.to_string(),
    }
}

/// UTC milliseconds.
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}
