use sqlx::SqlitePool;
use std::error::Error;
use tempfile::TempDir;

/// Concurrent readers must not block background writes, so use the app's WAL configuration.
/// Callers close the directory after the pool so cleanup failures cannot be ignored.
pub async fn open() -> Result<(TempDir, SqlitePool), Box<dyn Error + Send + Sync>> {
    let dir = tempfile::tempdir()?;
    let db = ::db::open(&dir.path().join("app.db")).await?;
    Ok((dir, db))
}

/// Runs `work`, which is to wait in place while the database fails, until `writes` says so; then
/// has `fix` make the database work again and lets the five seconds before the next try pass.
/// What `work` then finishes with, once nothing is failing any more.
pub async fn waits_for_storage<T>(
    writes: &::db::Writes,
    work: impl Future<Output = T>,
    fix: impl Future<Output = ()>,
) -> T {
    // Keeps paused time from jumping ahead while the database works on its own threads.
    let busy = tokio::spawn(async {
        loop {
            tokio::task::yield_now().await;
        }
    });
    tokio::time::pause();
    tokio::pin!(work);
    // Paused time does not pass, so the clock on the wall bounds each wait.
    let within = |seconds| {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
        async move {
            while std::time::Instant::now() < end {
                tokio::task::yield_now().await;
            }
        }
    };
    tokio::select! {
        _ = &mut work => panic!("gave up instead of waiting for the database"),
        () = async {
            while writes.failure().is_none() {
                tokio::task::yield_now().await;
            }
        } => {}
        () = within(30) => panic!("did not wait for the database within 30 seconds"),
    }
    fix.await;
    tokio::time::advance(std::time::Duration::from_secs(5)).await;
    tokio::time::resume();
    let done = tokio::select! {
        done = &mut work => done,
        () = within(30) => panic!("did not finish within 30 seconds once the database worked"),
    };
    busy.abort();
    assert_eq!(writes.failure(), None);
    done
}
