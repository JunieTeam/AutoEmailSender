//! Native dialogs and serialization of the local backup workflow.

use std::sync::{Mutex, PoisonError};

use api::CallError;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

#[derive(Default)]
pub struct Commands(Mutex<()>);

/// Runs `work` away from the async executor, one backup command at a time, logging its error once
/// under `command`.
async fn blocking<T: Send + 'static>(
    app: AppHandle,
    command: &str,
    work: impl FnOnce(&AppHandle) -> Result<T, CallError> + Send + 'static,
) -> Result<T, CallError> {
    let result = tauri::async_runtime::spawn_blocking(move || {
        let commands = app.state::<Commands>();
        // Guards no data: each command leaves the files it works on safe to go on with.
        let _held = commands.0.lock().unwrap_or_else(PoisonError::into_inner);
        work(&app)
    })
    .await
    .map_err(CallError::internal);
    crate::logged(command, result.and_then(|result| result))
}

#[tauri::command]
pub async fn export_backup(
    app: AppHandle,
    ctx: tauri::State<'_, std::sync::Arc<api::Context>>,
) -> Result<bool, CallError> {
    let db = ctx.db.clone();
    blocking(app, "export_backup", move |app| {
        let dir = crate::data(app)?;
        let now = now();
        let date: String = tauri::async_runtime::block_on(
            sqlx::query_scalar("SELECT date(? / 1000, 'unixepoch', 'localtime')")
                .bind(now)
                .fetch_one(&db),
        )?;
        let Some(target) = app
            .dialog()
            .file()
            .add_filter("Auto Email Sender 备份", &["zip"])
            .set_file_name(format!("Auto Email Sender 备份 {date}.zip"))
            .blocking_save_file()
        else {
            return Ok(false);
        };
        let target = target.into_path().map_err(CallError::internal)?;
        tauri::async_runtime::block_on(api::backup::export(&db, &dir, &target, now))?;
        tracing::info!("exported a backup");
        Ok(true)
    })
    .await
}

#[tauri::command]
pub async fn prepare_restore(
    app: AppHandle,
) -> Result<Option<api::backup::RestoreInfo>, CallError> {
    blocking(app, "prepare_restore", |app| {
        let dir = crate::data(app)?;
        // Opening a second picker abandons the previous unconfirmed choice, including on cancel.
        api::backup::cancel(&dir)?;
        let Some(source) = app
            .dialog()
            .file()
            .add_filter("Auto Email Sender 备份", &["zip"])
            .blocking_pick_file()
        else {
            return Ok(None);
        };
        let source = source.into_path().map_err(CallError::internal)?;
        let info = tauri::async_runtime::block_on(api::backup::prepare(&dir, &source))?;
        tracing::info!(
            exported_at = info.exported_at,
            "prepared a restore from a file"
        );
        Ok(Some(info))
    })
    .await
}

/// Downloads the cloud backup into the same staged restore that `restore_backup` confirms.
#[tauri::command]
pub async fn prepare_cloud_restore(
    app: AppHandle,
    ctx: tauri::State<'_, std::sync::Arc<api::Context>>,
) -> Result<api::backup::RestoreInfo, CallError> {
    let ctx = ctx.inner().clone();
    blocking(app, "prepare_cloud_restore", move |app| {
        let dir = crate::data(app)?;
        let info = tauri::async_runtime::block_on(api::backup::cloud::prepare_restore(&ctx, &dir))?;
        tracing::info!(
            exported_at = info.exported_at,
            "prepared a restore from the cloud"
        );
        Ok(info)
    })
    .await
}

#[tauri::command]
pub async fn cancel_restore(app: AppHandle) -> Result<(), CallError> {
    blocking(app, "cancel_restore", |app| {
        Ok(api::backup::cancel(&crate::data(app)?)?)
    })
    .await
}

#[tauri::command]
pub async fn restore_backup(
    app: AppHandle,
    ctx: tauri::State<'_, std::sync::Arc<api::Context>>,
    exported_at: i64,
) -> Result<(), CallError> {
    let db = ctx.db.clone();
    blocking(app.clone(), "restore_backup", move |app| {
        let dir = crate::data(app)?;
        Ok(tauri::async_runtime::block_on(api::backup::confirm(
            &db,
            &dir,
            exported_at,
            now(),
        ))?)
    })
    .await?;
    tracing::info!(exported_at, "restore confirmed; restarting to install it");
    app.restart();
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}
