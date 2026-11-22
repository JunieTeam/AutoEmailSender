// Release builds on Windows run without a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod background;
// On the modules, not each command: the check sits beside the function, where its attributes do
// not reach.
#[expect(
    clippy::unreachable,
    reason = "#[tauri::command] checks with unreachable!() that async commands borrowing arguments return a Result"
)]
mod backup;
mod browser;
#[expect(
    clippy::unreachable,
    reason = "#[tauri::command] checks with unreachable!() that async commands borrowing arguments return a Result"
)]
mod commands;
mod logs;
mod notifications;
mod save;
mod updater;

use std::error::Error;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use api::CallError;
use api::startup::StartupFailure;
use chrono::DateTime;
use failure::Failure;
use tauri::{AppHandle, Emitter, Listener, Manager};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

/// Logs a command's error once, as it leaves the command (docs/development.md#错误和日志).
fn logged<T>(command: &str, result: Result<T, CallError>) -> Result<T, CallError> {
    result.inspect_err(|e| e.record(command, api::Caller::Window))
}

/// Reading or writing a file failed for a reason of the system's, after `what` failed.
fn file_failed(what: &str, e: std::io::Error) -> CallError {
    CallError::external(failure::file(what, &e), e)
}

fn main() -> tauri::Result<()> {
    let context = tauri::generate_context!();
    let app = tauri::Builder::default()
        // First, so that launching the app again only brings the running one forward.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| show(app)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name(context.config().identifier.clone())
                .arg(autostart::ARG)
                .build(),
        )
        .manage(backup::Commands::default())
        .manage(updater::Updater::default())
        .setup(|app| {
            background::init(app.handle())?;
            // A login launch stays in the tray unless there is something the user must see.
            let must_show = match tauri::async_runtime::block_on(start(app.handle())) {
                Ok((ctx, restored)) => {
                    app.manage(ctx);
                    updater::start(app.handle());
                    restored
                }
                Err(e) => {
                    tracing::error!("startup failed: {e}");
                    app.manage(StartupFailure {
                        reason: e.failure(),
                        diagnostic: failure::scrub(&e.to_string()),
                    });
                    true
                }
            };
            if must_show || !std::env::args_os().any(|arg| arg == autostart::ARG) {
                show(app.handle());
            }
            Ok(())
        })
        .on_window_event(background::on_window_event)
        .invoke_handler(tauri::generate_handler![
            commands::call,
            commands::startup_failure,
            commands::export_logs,
            commands::export_crawl_log,
            commands::export_mentors,
            backup::export_backup,
            backup::prepare_restore,
            backup::prepare_cloud_restore,
            backup::cancel_restore,
            backup::restore_backup,
            commands::data_location,
            commands::open_data_location,
            autostart::autostart_enabled,
            autostart::set_autostart,
            notifications::notification_permission,
            notifications::request_notification_permission,
            #[cfg(target_os = "macos")]
            notifications::open_notification_settings,
            updater::changelog,
            updater::update_status,
            updater::check_update,
            updater::download_update,
            updater::restart_update
        ])
        .build(context)?;
    app.run(|app, event| {
        if matches!(
            event,
            tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
        ) {
            before_exit(app);
        }
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen { .. } = event {
            show(app);
        }
    });
    Ok(())
}

fn before_exit(app: &AppHandle) {
    browser::close_all(app);
}

/// Opens the data directory, the logs and the database.
async fn start(app: &AppHandle) -> Result<(Arc<api::Context>, bool), StartupError> {
    let dir = data_dir(app).map_err(|e| StartupError::DataDir(e.into()))?;
    let lock = match api::startup::DataLock::acquire(&dir) {
        Ok(lock) => lock,
        // The single-instance plugin can miss simultaneous launches.
        Err(api::startup::Error::InUse) => std::process::exit(0),
        Err(e) => return Err(StartupError::Core(e)),
    };
    app.manage(lock);
    logs::init(&dir.join("logs")).map_err(StartupError::Logs)?;
    tracing::info!(
        "starting {} on {} {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    if let Err(e) = autostart::refresh(app) {
        tracing::error!("refresh autostart: {e}");
    }
    let (ctx, restored) = api::startup::open(&dir).await.map_err(StartupError::Core)?;
    let ctx = Arc::new(ctx);
    let mut account_changed = ctx.account.subscribe();
    let ui = app.clone();
    tauri::async_runtime::spawn(async move {
        while account_changed.changed().await.is_ok() {
            refresh(&ui, "account-changed");
        }
    });
    let ui = app.clone();
    let agent = api::agent::start(ctx.clone(), dir, move |event| match event {
        api::agent::Event::Requested(description) => {
            refresh(&ui, "agent-requests-changed");
            show(&ui);
            if let Err(e) = ui
                .notification()
                .builder()
                .title("Agent 请求确认")
                .body(&description.action)
                .show()
            {
                tracing::warn!("agent confirmation notification: {e}");
            }
        }
        api::agent::Event::RequestsChanged => refresh(&ui, "agent-requests-changed"),
        api::agent::Event::Called => refresh(&ui, "called-elsewhere"),
    })
    .await
    .map_err(StartupError::Agent)?;
    app.manage(agent);
    let ui = app.clone();
    api::phones::start(ctx.clone(), move |event| match event {
        api::phones::Event::Pairing(name) => {
            if name.is_some() {
                show(&ui);
            }
            if let Err(e) = ui.emit("phone-pairing", name) {
                tracing::warn!("ask about a phone: {e}");
            }
        }
        api::phones::Event::Changed => refresh(&ui, "phones-changed"),
        api::phones::Event::Called => refresh(&ui, "called-elsewhere"),
    });
    // Phones show what the window shows. Calls from elsewhere are passed on where they are made,
    // so that a phone does not hear about its own.
    for event in [
        "account-changed",
        "sending-changed",
        "received",
        "reply-processing-changed",
        "todos-changed",
        "matching-changed",
        "tasks-changed",
        "crawl-changed",
        "crawl-finished",
        "drafts-finished",
        "matching-finished",
        "tracking-changed",
    ] {
        let ctx = ctx.clone();
        app.listen_any(event, move |e| ctx.phones.push(event, e.payload()));
    }
    tauri::async_runtime::spawn(api::libraries::run(ctx.clone()));
    tauri::async_runtime::spawn(api::backup::cloud::run(ctx.clone()));
    let ui = app.clone();
    tauri::async_runtime::spawn(api::cloud_send::run(ctx.clone(), move || {
        refresh(&ui, "sending-changed")
    }));
    let ui = app.clone();
    tauri::async_runtime::spawn(api::tracking::run(ctx.clone(), move || {
        refresh(&ui, "tracking-changed")
    }));
    let api::Context {
        writes,
        db,
        net,
        materials,
        sender,
        receiver,
        matcher,
        reply_processor,
        todo_reminders,
        crawler,
        generator,
        turns,
        ..
    } = &*ctx;
    let ui = app.clone();
    let tracking = ctx.clone();
    tauri::async_runtime::spawn(send::run(
        db.clone(),
        writes.clone(),
        net.clone(),
        materials.clone(),
        sender.clone(),
        move || {
            tracking.tracking.sent();
            refresh(&ui, "sending-changed");
        },
    ));
    let ui = app.clone();
    let notifications_db = db.clone();
    let received_processor = reply_processor.clone();
    tauri::async_runtime::spawn(receive::run(
        db.clone(),
        net.clone(),
        receiver.clone(),
        // The UI reads again what new mail may have changed.
        move |replies| {
            received_processor.wake();
            refresh(&ui, "received");
            if !replies.is_empty() {
                let db = notifications_db.clone();
                let app = ui.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = notify_replies(&app, &db, replies).await {
                        tracing::warn!("reply notification: {e}");
                    }
                });
            }
        },
    ));
    let ui = app.clone();
    let changed_todos = todo_reminders.clone();
    tauri::async_runtime::spawn(receive::outcome::run(
        db.clone(),
        writes.clone(),
        net.clone(),
        reply_processor.clone(),
        turns.clone(),
        move || {
            changed_todos.wake();
            refresh(&ui, "reply-processing-changed");
        },
    ));
    let (ui, notifications) = (app.clone(), app.clone());
    tauri::async_runtime::spawn(receive::reminders::run(
        db.clone(),
        writes.clone(),
        todo_reminders.clone(),
        move || refresh(&ui, "todos-changed"),
        move |message| {
            if let Err(e) = notifications
                .notification()
                .builder()
                .title("待办提醒")
                .body(message)
                .show()
            {
                tracing::warn!("todo notification: {e}");
            }
        },
    ));
    let (ui, done_ui) = (app.clone(), app.clone());
    let matching_notifications_db = db.clone();
    tauri::async_runtime::spawn(matching::run(
        db.clone(),
        writes.clone(),
        net.clone(),
        matcher.clone(),
        turns.clone(),
        move || refresh(&ui, "matching-changed"),
        move |result| {
            let app = done_ui.clone();
            let db = matching_notifications_db.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = finish_matching(&app, &db, result).await {
                    tracing::warn!("match completion notification: {e}");
                }
            });
        },
    ));
    let (ui, done_ui) = (app.clone(), app.clone());
    let drafts_db = db.clone();
    tauri::async_runtime::spawn(task::generation::run(
        db.clone(),
        writes.clone(),
        net.clone(),
        generator.clone(),
        turns.clone(),
        move || refresh(&ui, "tasks-changed"),
        move |result| {
            let (app, db) = (done_ui.clone(), drafts_db.clone());
            tauri::async_runtime::spawn(async move {
                if let Err(e) = finish_drafts(&app, &db, result).await {
                    tracing::warn!("draft completion notification: {e}");
                }
            });
        },
    ));
    let (ui, done_ui, crawl_db) = (app.clone(), app.clone(), db.clone());
    tauri::async_runtime::spawn(crawl::runner::run(
        db.clone(),
        net.clone(),
        crawler.clone(),
        turns.clone(),
        Some(std::sync::Arc::new(browser::Browser::new(app.clone()))),
        move || refresh(&ui, "crawl-changed"),
        move |result| {
            let (app, db) = (done_ui.clone(), crawl_db.clone());
            tauri::async_runtime::spawn(async move {
                if let Err(e) = finish_crawl(&app, &db, result).await {
                    tracing::warn!("crawl completion notification: {e}");
                }
            });
        },
    ));
    Ok((ctx, restored))
}

/// Tells the window to read again what `event` names.
fn refresh(ui: &AppHandle, event: &str) {
    if let Err(e) = ui.emit(event, ()) {
        tracing::warn!(event, "tell the window: {e}");
    }
}

async fn finish_crawl(
    app: &AppHandle,
    db: &sqlx::SqlitePool,
    result: crawl::runner::CrawlFinished,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    if result.message.is_empty() {
        return Ok(());
    }
    if let Some(window) = app.get_webview_window("main")
        && window.is_focused()?
    {
        app.emit("crawl-finished", result)?;
    } else if profile::background_notifications(db).await? {
        app.notification().builder().title(&result.message).show()?;
    }
    Ok(())
}
async fn finish_drafts(
    app: &AppHandle,
    db: &sqlx::SqlitePool,
    result: task::generation::DraftsFinished,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(window) = app.get_webview_window("main")
        && window.is_focused()?
    {
        app.emit("drafts-finished", result)?;
    } else if profile::background_notifications(db).await? {
        app.notification()
            .builder()
            .title(format!(
                "任务“{}”的草稿已生成：{} 封，失败 {} 封",
                result.name, result.generated, result.failed
            ))
            .show()?;
    }
    Ok(())
}

async fn finish_matching(
    app: &AppHandle,
    db: &sqlx::SqlitePool,
    result: matching::MatchFinished,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(window) = app.get_webview_window("main")
        && window.is_focused()?
    {
        app.emit("matching-finished", result)?;
    } else if profile::background_notifications(db).await? {
        let failed = if result.failed > 0 {
            format!("，失败 {} 位", result.failed)
        } else {
            String::new()
        };
        app.notification()
            .builder()
            .title(format!(
                "匹配度分析完成：分析了 {} 位{failed}",
                result.analyzed
            ))
            .show()?;
    }
    Ok(())
}

async fn notify_replies(
    app: &AppHandle,
    db: &sqlx::SqlitePool,
    replies: Vec<receive::NewReply>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    if !profile::reply_notifications(db).await? {
        return Ok(());
    }
    if let Some(window) = app.get_webview_window("main")
        && window.is_focused()?
    {
        return Ok(());
    }
    let (title, body) = if replies.len() == 1 && replies[0].mentors.len() == 1 {
        (
            format!("{}已回信", replies[0].mentors[0]),
            replies[0]
                .subject
                .clone()
                .unwrap_or_else(|| "（无主题）".into()),
        )
    } else {
        let mut names = Vec::new();
        for name in replies.iter().flat_map(|r| &r.mentors) {
            if !names.contains(name) {
                names.push(name.clone());
            }
        }
        (format!("{} 封新回信", replies.len()), names.join("、"))
    };
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()?;
    Ok(())
}

#[derive(Debug)]
enum StartupError {
    DataDir(Box<dyn Error>),
    Logs(Box<dyn Error>),
    Core(api::startup::Error),
    Agent(api::CallError),
}

impl std::fmt::Display for StartupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartupError::DataDir(e) => write!(f, "data directory: {e}"),
            StartupError::Logs(e) => write!(f, "logs: {e}"),
            StartupError::Core(e) => write!(f, "{e}"),
            StartupError::Agent(e) => write!(f, "agent: {e}"),
        }
    }
}

impl StartupError {
    /// What the user reads: what happened, and what was kept safe.
    fn failure(&self) -> Failure {
        match self {
            StartupError::DataDir(e) => Failure::unrecognized("数据目录无法使用", &e.to_string()),
            StartupError::Logs(e) => Failure::unrecognized("日志无法写入", &e.to_string()),
            StartupError::Core(e) => e.failure(),
            StartupError::Agent(e) => Failure {
                message: format!("Agent 连接无法启动：{}", e.message),
                details: e.details.clone(),
            },
        }
    }
}

/// Brings the main window forward when the app is launched again.
fn show(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if let Err(e) = window
        .unminimize()
        .and_then(|()| window.show())
        .and_then(|()| window.set_focus())
    {
        tracing::warn!("bring window forward: {e}");
    }
}

/// The data directory, for a command: one runs only once the directory was found at startup.
fn data(app: &AppHandle) -> Result<PathBuf, CallError> {
    data_dir(app).map_err(|e| CallError::internal(format!("data directory: {e}")))
}

/// Development builds keep their data in the repository, away from an installed app's data.
fn data_dir(app: &AppHandle) -> tauri::Result<PathBuf> {
    if cfg!(debug_assertions) {
        // crates/app is two levels below the repository.
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2);
        Ok(repo.ok_or(tauri::Error::UnknownPath)?.join(".dev-data"))
    } else {
        app.path().app_data_dir()
    }
}
