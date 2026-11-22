//! The window's commands that belong to no other part of the app.

use super::*;

/// The UI's one entry point into the core; see `api::call`.
#[tauri::command]
pub async fn call(
    ctx: tauri::State<'_, Arc<api::Context>>,
    op: String,
    input: serde_json::Value,
) -> Result<serde_json::Value, api::CallError> {
    api::call(&ctx, &op, input)
        .await
        .inspect_err(|e| e.record(&op, api::Caller::Window))
}

/// Why the core could not start, or None when it is running.
#[tauri::command]
pub fn startup_failure(app: AppHandle) -> Option<StartupFailure> {
    app.try_state::<StartupFailure>().map(|f| f.inner().clone())
}

/// Saves the logs where the user picks; false when they cancel.
#[tauri::command]
pub async fn export_logs(app: AppHandle) -> Result<bool, CallError> {
    // Missing only when starting up failed before the logs began, which the diagnostic says.
    let dir = data_dir(&app)
        .ok()
        .map(|dir| dir.join("logs"))
        .filter(|dir| dir.is_dir());
    let startup = app.try_state::<StartupFailure>();
    let diagnostic = startup.as_ref().map(|f| f.diagnostic.as_str());
    logged(
        "export_logs",
        logs::export(&app, dir.as_deref(), diagnostic),
    )
}

#[tauri::command]
pub async fn export_crawl_log(
    app: AppHandle,
    ctx: tauri::State<'_, Arc<api::Context>>,
    id: i64,
) -> Result<bool, CallError> {
    let exported = async {
        let detail = crawl::queue::detail(&ctx.db, id, 1, false)
            .await?
            .ok_or_else(|| CallError::not_found("这次整理已被删除"))?;
        let at = DateTime::from_timestamp_millis(detail.summary.created_at)
            .ok_or_else(|| CallError::internal(format!("crawl {id} started out of range")))?;
        let dir = data(&app)?.join("logs");
        tauri::async_runtime::spawn_blocking(move || logs::export_crawl(&app, &dir, &detail, at))
            .await
            .map_err(CallError::internal)?
    };
    logged("export_crawl_log", exported.await)
}

/// Saves the exported table through the native file picker, away from the async executor.
#[tauri::command]
pub async fn export_mentors(
    app: AppHandle,
    ctx: tauri::State<'_, Arc<api::Context>>,
    input: api::ExportMentors,
    file_name: String,
) -> Result<bool, CallError> {
    let extension = match input.format {
        api::TableFormat::Xlsx => "xlsx",
        api::TableFormat::Csv => "csv",
    };
    let exported = async {
        let data = api::export_mentors(&ctx, input).await?;
        tauri::async_runtime::spawn_blocking(move || {
            let Some(target) = app
                .dialog()
                .file()
                .add_filter("导师表格", &[extension])
                .set_file_name(format!("{file_name}.{extension}"))
                .blocking_save_file()
            else {
                return Ok(false);
            };
            save::picked(target, "表格未能保存", |file| file.write_all(&data))?;
            Ok(true)
        })
        .await
        .map_err(CallError::internal)?
    };
    logged("export_mentors", exported.await)
}

/// Where the app keeps its data, for the about page.
#[tauri::command]
pub fn data_location(app: AppHandle) -> Result<String, CallError> {
    logged(
        "data_location",
        data(&app).map(|dir| dir.display().to_string()),
    )
}

/// Opens the data directory in Finder or Explorer. The UI may not open paths itself.
#[tauri::command]
pub fn open_data_location(app: AppHandle) -> Result<(), CallError> {
    let opened = data(&app).and_then(|dir| {
        app.opener()
            .open_path(dir.display().to_string(), None::<&str>)
            .map_err(|e| {
                let failure = Failure::unrecognized(
                    "未能打开数据位置，请按上面显示的路径手动打开",
                    &e.to_string(),
                );
                CallError::external(failure, e)
            })
    });
    logged("open_data_location", opened)
}
