//! Opening the local core without starting background work.

use std::error::Error as StdError;
use std::fs::{File, TryLockError};
use std::path::Path;

use failure::Failure;
use serde::Serialize;
use ts_rs::TS;

use crate::Context;

/// Kept alive by the host so that no other host can write the same data directory.
pub struct DataLock(#[expect(dead_code, reason = "only kept open")] File);

impl DataLock {
    pub fn acquire(dir: &Path) -> Result<Self, Error> {
        std::fs::create_dir_all(dir).map_err(Error::DataDir)?;
        let file = File::options()
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join("app.lock"))
            .map_err(Error::DataDir)?;
        match file.try_lock() {
            Ok(()) => Ok(Self(file)),
            Err(TryLockError::WouldBlock) => Err(Error::InUse),
            Err(TryLockError::Error(e)) => Err(Error::DataDir(e)),
        }
    }
}

/// The host must hold the data directory lock before opening the core.
/// Also returns whether this startup installed a pending restore.
pub async fn open(dir: &Path) -> Result<(Context, bool), Error> {
    let data = dir.to_owned();
    let restored = tokio::task::spawn_blocking(move || crate::backup::apply_pending(&data))
        .await
        .map_err(|e| Error::Restore(crate::backup::Error::Worker(e)))?
        .map_err(Error::Restore)?;
    if restored {
        tracing::info!("installed the confirmed restore");
    }
    let db = db::open(&dir.join("app.db"))
        .await
        .map_err(Error::Database)?;
    let net = net::Net::new(fake_mail_root(dir).map_err(Error::DataDir)?).map_err(Error::Net)?;
    Ok((Context::new(db, net, dir), restored))
}

/// The authority of `cargo xtask fake`'s mail servers, which a development build trusts once the
/// command has run (docs/development.md#假邮箱和假模型).
fn fake_mail_root(
    dir: &Path,
) -> std::io::Result<Vec<net::rustls::pki_types::CertificateDer<'static>>> {
    if !cfg!(debug_assertions) {
        return Ok(Vec::new());
    }
    match std::fs::read(dir.join("fake-mail").join("root.der")) {
        Ok(der) => Ok(vec![der.into()]),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

#[derive(Debug)]
pub enum Error {
    InUse,
    DataDir(std::io::Error),
    Database(db::OpenError),
    Restore(crate::backup::Error),
    Net(Box<dyn StdError + Send + Sync>),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InUse => write!(f, "data directory already in use"),
            Self::DataDir(e) => write!(f, "data directory: {e}"),
            Self::Restore(e) => write!(f, "restore: {e}"),
            Self::Database(e) => write!(f, "{e}"),
            Self::Net(e) => write!(f, "network: {e}"),
        }
    }
}

impl StdError for Error {}

impl Error {
    pub fn failure(&self) -> Failure {
        use db::OpenError;
        match self {
            Self::InUse => Failure::new(
                "数据目录正在使用中，请先关闭这个工作树的桌面开发版或浏览器预览，再重试",
            ),
            Self::Restore(e) => {
                let reason = e.failure();
                Failure {
                    message: format!(
                        "恢复尚未完成，原数据已保留，请重启重试。{}。",
                        reason.message.trim_end_matches('。')
                    ),
                    details: reason.details,
                }
            }
            Self::DataDir(e) => failure::file("数据目录无法使用", e),
            Self::Net(e) => Failure::unrecognized("网络连接无法初始化", &e.to_string()),
            Self::Database(OpenError::Open(e)) => {
                Failure::unrecognized("数据库无法打开", &e.to_string())
            }
            Self::Database(OpenError::Backup(e)) => {
                Failure::unrecognized("升级数据库前未能备份，为保护数据未做升级", &e.to_string())
            }
            Self::Database(OpenError::Upgrade {
                error,
                backup: Some(path),
            }) => Failure::unrecognized(
                format!(
                    "数据库升级失败。升级前的数据库已备份到 {}。",
                    path.display()
                ),
                &error.to_string(),
            ),
            Self::Database(OpenError::Upgrade {
                error,
                backup: None,
            }) => Failure::unrecognized("数据库创建失败", &error.to_string()),
        }
    }
}

/// What the window shows in place of the app when it could not start (docs/ui.md#页面).
#[derive(Debug, Clone, Serialize, TS)]
pub struct StartupFailure {
    pub reason: Failure,
    /// What failed and the words it failed with, scrubbed: for the user to copy, and added to
    /// the exported logs, which may not have been written.
    pub diagnostic: String,
}
