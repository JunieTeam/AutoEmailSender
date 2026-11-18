//! Values whose rules only this app's code holds, such as a provider's id or an error saved as
//! JSON. The schema cannot list them, so a backup edited by hand would restore and then fail
//! wherever the app reads them, without saying why: they are checked as the app reads or saves them.

use std::borrow::Cow;

use failure::Failure;
use sqlx::sqlite::Sqlite;
use sqlx::types::Json;
use sqlx::{AssertSqlSafe, Row, SqliteConnection};

use super::{Error, Result};

pub(super) async fn inspect(conn: &mut SqliteConnection) -> Result<()> {
    for (table, column) in [
        ("mailbox", "pause_reason"),
        ("mailbox", "receive_error"),
        ("mailbox", "cloud_pause_reason"),
        ("profile", "match_pause_reason"),
        ("profile", "reply_processing_pause_reason"),
        ("profile", "cloud_backup_error"),
        ("mentor", "match_error"),
        ("task", "generation_pause_reason"),
        ("message_mentor", "bounce_reason"),
        ("reply_processing", "error"),
        ("email", "error"),
        ("crawl", "error"),
        ("crawl_page", "error"),
        ("candidate", "enrich_error"),
    ] {
        decodes::<Json<Failure>>(conn, table, column).await?;
    }
    for (table, column) in [
        ("mentor_detail", "papers"),
        ("mentor_detail", "match_directions"),
        ("mentor_detail", "match_highlights"),
        ("mentor_detail", "match_risks"),
        ("candidate", "aliases"),
        ("candidate", "emails"),
        ("candidate", "homepages"),
        ("candidate", "papers"),
        ("crawl", "start_urls"),
        ("crawl", "limits"),
    ] {
        decodes::<Json<Vec<String>>>(conn, table, column).await?;
    }
    decodes::<Json<Vec<mentor::Affiliation>>>(conn, "candidate", "affiliations").await?;
    decodes::<Json<mentor::MentorFields>>(conn, "candidate", "profile_fields").await?;
    decodes::<Json<Vec<crawl::review::Evidence>>>(conn, "candidate", "evidence").await?;
    decodes::<Json<Vec<crawl::enrich::EnrichChange>>>(conn, "candidate", "enrich_changes").await?;
    decodes::<Json<Vec<crawl::store::PageStep>>>(conn, "crawl_page", "steps").await?;
    decodes::<crawl::PageUrl>(conn, "crawl_page", "url").await?;
    decodes::<crawl::PageUrl>(conn, "crawl_page", "scope_url").await?;
    decodes::<Json<email::fact_check::FactCheck>>(conn, "email", "fact_check").await?;
    decodes::<Json<Vec<i64>>>(conn, "email", "open_times").await?;
    decodes::<Json<cloud_api::library::Record>>(conn, "mentor_library", "record").await?;
    models(conn).await?;
    mailboxes(conn).await?;
    schedules(conn).await
}

/// Each value of the column reads as `T`.
async fn decodes<T>(conn: &mut SqliteConnection, table: &str, column: &str) -> Result<()>
where
    T: for<'r> sqlx::Decode<'r, Sqlite> + sqlx::Type<Sqlite>,
{
    // The names are this file's own, never the backup's.
    let sql = format!("SELECT {column} FROM {table} WHERE {column} IS NOT NULL");
    for row in sqlx::query(AssertSqlSafe(sql))
        .fetch_all(&mut *conn)
        .await?
    {
        if row.try_get::<T, _>(0).is_err() {
            return Err(unreadable(&format!("{table} 表的 {column}")));
        }
    }
    Ok(())
}

/// Model configurations pass the checks made when they are saved.
async fn models(conn: &mut SqliteConnection) -> Result<()> {
    let rows = sqlx::query!(
        r#"SELECT id, name, provider, region, base_url, model,
             currency AS "currency: model::pricing::Currency", input_price, cache_price,
             output_price, off_peak_input_price, off_peak_cache_price, off_peak_output_price
           FROM model_config"#
    )
    .fetch_all(&mut *conn)
    .await?;
    for r in rows {
        let form = model::ModelForm {
            name: r.name.unwrap_or_default(),
            provider: r.provider,
            region: r.region,
            base_url: r.base_url.unwrap_or_default(),
            model: r.model,
            currency: Some(r.currency),
            prices: model::pricing::Prices {
                input: r.input_price,
                cache: r.cache_price,
                output: r.output_price,
            },
            off_peak: model::pricing::Prices {
                input: r.off_peak_input_price,
                cache: r.off_peak_cache_price,
                output: r.off_peak_output_price,
            },
        };
        if let Err(reason) = form.check() {
            return Err(unreadable(&format!("模型配置第 {} 条：{reason}", r.id)));
        }
    }
    Ok(())
}

/// Mailboxes pass the checks made when they are saved.
async fn mailboxes(conn: &mut SqliteConnection) -> Result<()> {
    let rows = sqlx::query!(
        r#"SELECT id, address, sender_name, provider, smtp_host, smtp_port,
             smtp_security AS "smtp_security: transport::Security", imap_host, imap_port,
             imap_security AS "imap_security: transport::Security", daily_limit, min_interval_s
           FROM mailbox"#
    )
    .fetch_all(&mut *conn)
    .await?;
    let out_of_range = |_| "数字超出范围".to_owned();
    for r in rows {
        let checked = (|| {
            mailbox::MailboxForm {
                address: r.address.clone(),
                sender_name: r.sender_name,
                provider: r.provider,
                smtp: transport::Server {
                    host: Cow::Owned(r.smtp_host),
                    port: u16::try_from(r.smtp_port).map_err(out_of_range)?,
                    security: r.smtp_security,
                },
                imap: transport::Server {
                    host: Cow::Owned(r.imap_host),
                    port: u16::try_from(r.imap_port).map_err(out_of_range)?,
                    security: r.imap_security,
                },
                daily_limit: u32::try_from(r.daily_limit).map_err(out_of_range)?,
                min_interval_s: u32::try_from(r.min_interval_s).map_err(out_of_range)?,
            }
            .check()
        })();
        if let Err(reason) = checked {
            return Err(unreadable(&format!("邮箱 {}：{reason}", r.address)));
        }
    }
    Ok(())
}

/// Sending windows read as the tasks sending in them do.
async fn schedules(conn: &mut SqliteConnection) -> Result<()> {
    for task in sqlx::query!("SELECT id, name FROM task")
        .fetch_all(&mut *conn)
        .await?
    {
        if email::schedule::get(conn, task.id).await.is_err() {
            return Err(unreadable(&format!("任务“{}”的发送时间", task.name)));
        }
    }
    Ok(())
}

fn unreadable(place: &str) -> Error {
    Error::Invalid(format!(
        "备份里有本应用读不了的数据（{place}），可能被手动改过，请换一份备份文件"
    ))
}
