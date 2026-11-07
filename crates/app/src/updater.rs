//! Updates stay in the application process, including when all windows are hidden.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager, Url};
use tauri_plugin_updater::{Error, Update, UpdaterExt};

use api::updater::{GITHUB_MANIFEST, MIRROR, UpdateInfo, UpdateStatus};

const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// For each manifest in turn: GitHub's, then the mirror's.
const CHECK_TIMEOUT: Duration = Duration::from_secs(15);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// How long a download from GitHub runs before its pace is judged.
const PATIENCE: Duration = Duration::from_secs(20);
/// A download from GitHub whose pace would take longer than this in all moves to the mirror.
const SLOW_DOWNLOAD: Duration = Duration::from_secs(5 * 60);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Default)]
pub struct Updater {
    phase: Mutex<Phase<Update>>,
    operation: tokio::sync::Mutex<()>,
}

#[derive(Default)]
enum Phase<U> {
    #[default]
    Current,
    Available {
        update: U,
        error: Option<String>,
    },
    Downloading {
        update: U,
        downloaded: u64,
        total: Option<u64>,
    },
    Ready {
        update: U,
        bytes: Arc<[u8]>,
    },
}

impl<U: Clone> Phase<U> {
    fn begin_download(&mut self) -> Result<U, String> {
        let Self::Available { update, .. } = self else {
            return Err("当前无可下载的更新，或更新正在下载".into());
        };
        let update = update.clone();
        *self = Self::Downloading {
            update: update.clone(),
            downloaded: 0,
            total: None,
        };
        Ok(update)
    }

    fn set_progress(&mut self, bytes: u64, size: Option<u64>) {
        if let Self::Downloading {
            downloaded, total, ..
        } = self
        {
            *downloaded = bytes;
            *total = size;
        }
    }

    /// Whether the download, at the pace it has kept for `PATIENCE`, would take longer than
    /// `SLOW_DOWNLOAD` in all.
    fn too_slow(&self) -> bool {
        let Self::Downloading {
            downloaded, total, ..
        } = *self
        else {
            unreachable!("only a download in progress is timed")
        };
        total.map_or(downloaded == 0, |total| {
            u128::from(total) * PATIENCE.as_millis()
                > u128::from(downloaded) * SLOW_DOWNLOAD.as_millis()
        })
    }

    fn finish_download(&mut self, update: U, result: Result<Vec<u8>, String>) {
        *self = match result {
            Ok(bytes) => Self::Ready {
                update,
                bytes: bytes.into(),
            },
            Err(error) => Self::Available {
                update,
                error: Some(error),
            },
        };
    }

    fn installation(&self) -> Result<(U, Arc<[u8]>), String> {
        match self {
            Self::Ready { update, bytes } => Ok((update.clone(), bytes.clone())),
            _ => Err("更新尚未下载完成，请先下载再重启".into()),
        }
    }
}

impl Updater {
    fn phase(&self) -> MutexGuard<'_, Phase<Update>> {
        self.phase.lock().unwrap_or_else(|_| {
            unreachable!("update state changes cannot panic while holding the lock")
        })
    }

    fn status(&self) -> UpdateStatus {
        let info = |update: &Update| UpdateInfo {
            version: update.version.clone(),
            published_at: update.date.map(|date| date.unix_timestamp() * 1000),
            notes: update.body.clone(),
        };
        match &*self.phase() {
            Phase::Current => UpdateStatus::Current,
            Phase::Available { update, error } => UpdateStatus::Available {
                update: info(update),
                error: error.clone(),
            },
            Phase::Downloading {
                update,
                downloaded,
                total,
            } => UpdateStatus::Downloading {
                update: info(update),
                downloaded: *downloaded,
                total: *total,
            },
            Phase::Ready { update, .. } => UpdateStatus::Ready {
                update: info(update),
            },
        }
    }
}

fn changed(app: &AppHandle) {
    if let Err(e) = app.emit("update-changed", ()) {
        tracing::warn!("refresh update: {e}");
    }
}

pub fn start(app: &AppHandle) {
    if !cfg!(debug_assertions) {
        let app = app.clone();
        tauri::async_runtime::spawn(schedule(async move || {
            if let Err(error) = check(&app).await {
                tracing::warn!("automatic update check: {error}");
            }
        }));
    }
}

async fn schedule(mut check: impl AsyncFnMut()) {
    let mut ticks = tokio::time::interval(CHECK_INTERVAL);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        check().await;
    }
}

#[tauri::command]
pub fn update_status(app: AppHandle) -> UpdateStatus {
    app.state::<Updater>().status()
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<UpdateStatus, String> {
    check(&app).await
}

async fn check(app: &AppHandle) -> Result<UpdateStatus, String> {
    let state = app.state::<Updater>();
    if cfg!(debug_assertions) {
        return Ok(state.status());
    }
    let Ok(_operation) = state.operation.try_lock() else {
        return Err("正在检查、下载或安装更新，请稍后再试".into());
    };
    // Keep a downloaded package and its release together until the user installs it.
    if matches!(*state.phase(), Phase::Ready { .. }) {
        return Ok(state.status());
    }
    let result = updater(app, vec![url(GITHUB_MANIFEST), mirror_manifest()])?
        .check()
        .await
        .map_err(check_error)?;
    if let Some(update) = &result {
        tracing::info!(
            version = update.version,
            from = update.download_url.host_str(),
            "update available"
        );
    }
    *state.phase() = match result {
        Some(update) => Phase::Available {
            update,
            error: None,
        },
        None => Phase::Current,
    };
    changed(app);
    Ok(state.status())
}

/// The plugin's updater for `endpoints`, tried in order, through the app's network settings.
fn updater(app: &AppHandle, endpoints: Vec<Url>) -> Result<tauri_plugin_updater::Updater, String> {
    let net = app
        .try_state::<Arc<api::Context>>()
        .ok_or("应用尚未就绪，无法检查更新，请先处理启动错误")?
        .net
        .clone();
    let exit_app = app.clone();
    app.updater_builder()
        .endpoints(endpoints)
        .map_err(check_error)?
        .timeout(CHECK_TIMEOUT)
        .configure_client(move |builder| net.configure_http(builder))
        // Replaces the plugin's hook, which runs Tauri's cleanup_before_exit. That cannot be
        // undone, yet on Windows the hook runs before the installer starts, and the app goes on
        // when Windows refuses to start it. Hiding the tray icon is all the exit needs.
        .on_before_exit(move || {
            crate::before_exit(&exit_app);
            show_tray(&exit_app, false);
        })
        .build()
        .map_err(check_error)
}

fn url(address: &str) -> Url {
    Url::parse(address).unwrap_or_else(|e| unreachable!("{address} is a valid URL: {e}"))
}

fn mirror_manifest() -> Url {
    url(&format!("{MIRROR}latest.json"))
}

// Plugin errors can include remote JSON strings; only log the category and safe HTTP details.
fn check_error(error: Error) -> String {
    let reason = match error {
        Error::Reqwest(error) => {
            tracing::warn!(
                timeout = error.is_timeout(),
                status = ?error.status(),
                "update request failed"
            );
            if error.is_timeout() {
                "连接超时"
            } else {
                "无法连接更新服务或读取更新清单"
            }
        }
        _ => "更新配置或更新清单不正确",
    };
    format!("检查更新失败：{reason}，请检查网络和系统代理后重试")
}

#[tauri::command]
pub async fn download_update(app: AppHandle) -> Result<(), String> {
    let state = app.state::<Updater>();
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "正在检查、下载或安装更新，请稍后再试".to_owned())?;
    let mut update = state.phase().begin_download()?;
    update.timeout = Some(DOWNLOAD_TIMEOUT);
    changed(&app);
    let result = fetch(&app, &update).await.map_err(|error| {
        let (category, reason) = download_failure(&error);
        tracing::warn!(category, "update download failed");
        reason.to_owned()
    });
    let response = result.as_ref().map(|_| ()).map_err(Clone::clone);
    state.phase().finish_download(update, result);
    changed(&app);
    response
}

/// Downloads `update`. A download from GitHub that fails, or that turns out too slow after
/// `PATIENCE`, starts over from the mirror when the mirror has the same release.
async fn fetch(app: &AppHandle, update: &Update) -> Result<Vec<u8>, Error> {
    let mut github = Box::pin(update.download(progress(app), || {}));
    if update.download_url.as_str().starts_with(MIRROR) {
        return github.await;
    }
    let state = app.state::<Updater>();
    let failed = tokio::select! {
        result = &mut github => match result {
            Ok(bytes) => return Ok(bytes),
            Err(error) => Some(error),
        },
        () = tokio::time::sleep(PATIENCE) => {
            if !state.phase().too_slow() {
                return github.await;
            }
            None
        }
    };
    let mut mirror = match failed {
        Some(error) => {
            let (category, _) = download_failure(&error);
            tracing::warn!(category, "update download from GitHub failed");
            match mirror_release(app, &update.version).await {
                Some(mirror) => mirror,
                None => return Err(error),
            }
        }
        None => {
            tracing::info!("update download from GitHub is too slow");
            // GitHub goes on meanwhile, and is kept if it finishes first.
            tokio::select! {
                result = &mut github => return result,
                mirror = mirror_release(app, &update.version) => match mirror {
                    Some(mirror) => mirror,
                    None => return github.await,
                },
            }
        }
    };
    drop(github);
    tracing::info!("update download starts over from the mirror");
    state.phase().set_progress(0, None);
    changed(app);
    mirror.timeout = Some(DOWNLOAD_TIMEOUT);
    mirror.download(progress(app), || {}).await
}

/// The mirror's copy of `version`, when the mirror is up and has it.
async fn mirror_release(app: &AppHandle, version: &str) -> Option<Update> {
    let checked = match updater(app, vec![mirror_manifest()]) {
        Ok(updater) => updater.check().await.map_err(check_error),
        Err(error) => Err(error),
    };
    match checked {
        Ok(Some(update)) if update.version == version => Some(update),
        Ok(_) => {
            tracing::info!("update mirror has no copy of this release");
            None
        }
        Err(_) => {
            tracing::info!("update mirror unavailable");
            None
        }
    }
}

/// Shows how far a download, from its start, has got.
fn progress(app: &AppHandle) -> impl FnMut(usize, Option<u64>) + '_ {
    let mut downloaded = 0;
    let mut last_event = Instant::now();
    move |chunk, total| {
        downloaded += chunk as u64;
        app.state::<Updater>()
            .phase()
            .set_progress(downloaded, total);
        if last_event.elapsed() >= PROGRESS_INTERVAL {
            changed(app);
            last_event = Instant::now();
        }
    }
}

/// What went wrong, for the log, and what to tell the user.
fn download_failure(error: &Error) -> (&'static str, &'static str) {
    match error {
        Error::Minisign(_)
        | Error::Base64(_)
        | Error::SignatureUtf8(_)
        | Error::SignedVersionMismatch { .. }
        | Error::MissingSignedVersion => (
            "signature",
            "更新包签名校验失败，未安装，请重新下载；仍失败时请从官网下载",
        ),
        Error::Reqwest(error) if error.is_timeout() => {
            ("timeout", "更新下载超时，请检查网络和系统代理后重试")
        }
        Error::Reqwest(_) | Error::Network(_) => {
            ("network", "更新下载失败，请检查网络和系统代理后重试")
        }
        _ => ("package", "更新包无法读取或校验，未安装，请重新下载"),
    }
}

#[tauri::command]
pub async fn restart_update(app: AppHandle) -> Result<(), String> {
    let install_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = install_app.state::<Updater>();
        let _operation = state
            .operation
            .try_lock()
            .map_err(|_| "正在检查、下载或安装更新，请稍后再试".to_owned())?;
        let (update, bytes) = state.phase().installation()?;
        update.install(&*bytes).map_err(|error| {
            tracing::error!("update installation failed: {error}");
            show_tray(&install_app, true);
            "更新未能安装，请检查应用所在目录的权限后重试".to_owned()
        })?;
        install_app.restart();
    })
    .await
    .map_err(|error| format!("更新处理未能完成：{error}，请重试"))?
}

fn show_tray(app: &AppHandle, visible: bool) {
    if let Some(tray) = app.tray_by_id(crate::background::TRAY)
        && let Err(error) = tray.set_visible(visible)
    {
        tracing::warn!("show tray icon: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_failure_can_retry_and_only_verified_bytes_are_installable() -> Result<(), String> {
        let mut phase = Phase::Current;
        assert!(phase.installation().is_err());
        phase = Phase::Available {
            update: "3.0.1",
            error: None,
        };
        let update = phase.begin_download()?;
        assert!(phase.begin_download().is_err());
        assert!(phase.installation().is_err());
        phase.finish_download(update, Err("bad signature".into()));
        assert!(matches!(&phase, Phase::Available { error: Some(_), .. }));
        let update = phase.begin_download()?;
        phase.finish_download(update, Ok(vec![1, 2, 3]));
        let (version, bytes) = phase.installation()?;
        assert_eq!(version, "3.0.1");
        assert_eq!(&*bytes, &[1, 2, 3]);
        assert!(phase.begin_download().is_err());
        Ok(())
    }

    #[test]
    fn a_download_is_too_slow_when_its_pace_would_take_over_five_minutes() {
        let downloading = |downloaded, total| Phase::Downloading {
            update: (),
            downloaded,
            total,
        };
        // 20 seconds of a five-minute download of 20 MB bring 1⅓ MB.
        assert!(downloading(1_300_000, Some(20_000_000)).too_slow());
        assert!(!downloading(1_400_000, Some(20_000_000)).too_slow());
        assert!(downloading(0, None).too_slow());
        assert!(!downloading(1, None).too_slow());
    }

    #[tokio::test(start_paused = true)]
    async fn checks_at_startup_and_every_six_hours() {
        let (sent, mut received) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(schedule(async move || {
            sent.send(tokio::time::Instant::now()).unwrap();
        }));
        let first = received.recv().await.unwrap();
        tokio::time::advance(CHECK_INTERVAL - Duration::from_secs(1)).await;
        assert!(received.try_recv().is_err());
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(received.recv().await.unwrap() - first, CHECK_INTERVAL);
        tokio::time::advance(CHECK_INTERVAL).await;
        assert_eq!(received.recv().await.unwrap() - first, CHECK_INTERVAL * 2);
        task.abort();
    }
}
