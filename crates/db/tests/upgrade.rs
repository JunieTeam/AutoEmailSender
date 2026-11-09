//! Upgrades that rebuild a table keep its rows and everything that refers to them.
// Marks the whole file as test code, so clippy's test allowances cover it.
#![cfg(test)]

use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

#[tokio::test]
async fn rebuilding_the_email_table_keeps_emails_and_what_refers_to_them() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("rebuild-email");
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("app.db");
    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true);
    let old = SqlitePoolOptions::new()
        .connect_with(options)
        .await
        .unwrap();
    db::MIGRATOR.run_to(18, &old).await.unwrap();
    sqlx::raw_sql(
        r#"INSERT INTO mentor (id, name, created_at, updated_at) VALUES (1, '王伟', 0, 0);
        INSERT INTO email (id, mentor_id, kind, state, planned_at, latest_at, subject, body_md,
            html, text, created_at, updated_at)
          VALUES (1, 1, 'first', 'approved', 10, 20, '申请', '您好', '<p>您好</p>', '您好', 0, 0);
        INSERT INTO email (id, mentor_id, kind, state, planned_at, mailbox_id, from_address,
            to_address, message_id, sent_at, error, subject, body_md, html, text, created_at,
            updated_at, resend_of)
          VALUES (2, 1, 'first', 'bounced', 5, NULL, 'me@example.com', 'wang@example.edu',
            '<2@example.com>', 5, '退信', '申请', '您好', '<p>您好</p>', '您好', 0, 0, NULL);
        UPDATE email SET resend_of = 2 WHERE id = 1;
        INSERT INTO email_attachment (email_id, material_id, file_name, size)
          VALUES (1, NULL, '简历.pdf', 100);
        INSERT INTO usage (at, model, purpose, email_id, input_tokens, cached_tokens,
            output_tokens)
          VALUES (0, 'm', 'draft', 1, 1, 0, 1);"#,
    )
    .execute(&old)
    .await
    .unwrap();
    old.close().await;

    // As the app upgrades.
    let db = db::open(&path).await.unwrap();
    let emails: Vec<(i64, String, Option<i64>, Option<i64>)> =
        sqlx::query_as("SELECT id, state, latest_at, resend_of FROM email ORDER BY id")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(
        emails,
        [
            (1, "approved".into(), Some(20), Some(2)),
            (2, "bounced".into(), None, None)
        ]
    );
    let attached: i64 =
        sqlx::query_scalar("SELECT count(*) FROM email_attachment WHERE email_id = 1")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(attached, 1);
    let used: Option<i64> = sqlx::query_scalar("SELECT email_id FROM usage")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(used, Some(1));
    let broken: Vec<(String,)> = sqlx::query_as("PRAGMA foreign_key_check")
        .fetch_all(&db)
        .await
        .unwrap();
    assert!(broken.is_empty());

    // The triggers and views work on the new table, and foreign keys are enforced again.
    sqlx::query(
        r#"UPDATE email
        SET state = 'sent', sent_at = 30, mailbox_id = NULL, from_address = 'me@example.com',
            to_address = 'wang@example.edu', message_id = '<1@example.com>', latest_at = NULL
        WHERE id = 1"#,
    )
    .execute(&db)
    .await
    .unwrap();
    let out: Option<i64> =
        sqlx::query_scalar("SELECT last_out_at FROM mentor_contact WHERE mentor_id = 1")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(out, Some(30));
    let orphan = sqlx::query(
        r#"INSERT INTO email_attachment (email_id, material_id, file_name, size)
        VALUES (99, NULL, 'x', 1)"#,
    )
    .execute(&db)
    .await;
    assert!(orphan.is_err());
}

#[tokio::test]
async fn reasons_kept_as_text_become_failures_without_details() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("failure-reasons");
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("app.db");
    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true);
    let old = SqlitePoolOptions::new()
        .connect_with(options)
        .await
        .unwrap();
    db::MIGRATOR.run_to(20, &old).await.unwrap();
    sqlx::raw_sql(
        r#"INSERT INTO mentor (id, name, created_at, updated_at) VALUES (1, '王伟', 0, 0);
        INSERT INTO email (id, mentor_id, kind, state, planned_at, from_address, to_address,
            message_id, sent_at, error, subject, body_md, html, text, created_at, updated_at)
          VALUES (1, 1, 'first', 'bounced', 5, 'me@example.com', 'wang@example.edu',
            '<1@example.com>', 5, '收件地址不存在', '申请', '您好', '<p>您好</p>', '您好', 0, 0);
        INSERT INTO email (id, mentor_id, kind, state, subject, body_md, fact_check, created_at,
            updated_at)
          VALUES (2, 1, 'first', 'review', '申请', '您好',
            '{"state":"failed","reason":"模型未返回完整的核对结果"}', 0, 0);
        UPDATE profile SET match_paused_at = 1, match_pause_reason = '尚未添加模型';"#,
    )
    .execute(&old)
    .await
    .unwrap();
    old.close().await;

    let db = db::open(&path).await.unwrap();
    let reasons: (String, String, String) = sqlx::query_as(
        r#"SELECT (SELECT error FROM email WHERE id = 1),
            (SELECT fact_check FROM email WHERE id = 2), match_pause_reason
        FROM profile"#,
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(
        reasons,
        (
            r#"{"message":"收件地址不存在","details":null}"#.into(),
            r#"{"state":"failed","reason":{"message":"模型未返回完整的核对结果","details":null}}"#
                .into(),
            r#"{"message":"尚未添加模型","details":null}"#.into(),
        )
    );
}

#[tokio::test]
async fn rebuilding_the_crawl_table_gives_no_deleted_crawls_id_again() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("rebuild-crawl");
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("app.db");
    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true);
    let old = SqlitePoolOptions::new()
        .connect_with(options)
        .await
        .unwrap();
    db::MIGRATOR.run_to(24, &old).await.unwrap();
    sqlx::raw_sql(
        r#"INSERT INTO crawl (kind, start_urls, state, created_at)
          VALUES ('site', '["https://example.edu/a"]', 'running', 0),
            ('site', '["https://example.edu/b"]', 'running', 0),
            ('agent', NULL, 'running', 0);
        DELETE FROM crawl WHERE id > 1;"#,
    )
    .execute(&old)
    .await
    .unwrap();
    old.close().await;

    let db = db::open(&path).await.unwrap();
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO crawl (kind, state, created_at) VALUES ('mentors', 'running', 1) RETURNING id",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(id, 4);
}
