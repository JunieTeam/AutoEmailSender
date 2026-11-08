#![cfg(test)]

use api::Context;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use test_fixtures::feedback::Server;

struct Setup {
    dir: tempfile::TempDir,
    ctx: Context,
    server: Server,
}

impl Setup {
    async fn new(db: SqlitePool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("logs")).unwrap();
        let server = Server::start().await.unwrap();
        let mut ctx = Context::new(db, net::Net::new(vec![]).unwrap(), dir.path());
        ctx.account = account::Client::with_address(&ctx.net, server.address.clone());
        sqlx::raw_sql(
            r#"INSERT INTO account (terms, id, email, token)
            VALUES ('9999-12-31', 1, 'student@example.invalid', 'token')"#,
        )
        .execute(&ctx.db)
        .await
        .unwrap();
        let setup = Self { dir, ctx, server };
        setup.log("2026-11-07T08:00:00.000000Z  INFO app: starting for li@example.edu.cn");
        setup
    }

    fn log(&self, line: &str) {
        use std::io::Write;
        let path = self.dir.path().join("logs").join("app.2026-11-07.log");
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(file, "{line}").unwrap();
    }

    async fn call(&self, op: &str, input: Value) -> Result<Value, api::CallError> {
        api::call(&self.ctx, op, input).await
    }

    async fn submit(&self, description: &str, with_log: bool) -> Result<Value, api::CallError> {
        self.call(
            "submitFeedback",
            json!({ "description": description, "withLog": with_log }),
        )
        .await
    }

    fn answer(&self, answer: Option<(u16, &'static str)>) {
        self.server.state.lock().unwrap().answer = answer;
    }
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn feedback_tried_again_keeps_its_number_and_the_log_it_showed(db: SqlitePool) {
    let s = Setup::new(db).await;
    let shown = s.call("previewFeedback", json!({})).await.unwrap();
    let log = s.call("getFeedbackLog", json!({})).await.unwrap();
    let log = log.as_str().unwrap().to_owned();
    assert!(log.contains("starting for l***@example.edu.cn"), "{log}");
    assert_eq!(shown["size"], log.len());

    // The server got it but could not pass it on; what is logged meanwhile is not sent.
    s.answer(Some((503, "feedback_unavailable")));
    let error = s.submit(" 导出时卡住了\n", true).await.unwrap_err();
    assert_eq!(
        error.message,
        "反馈暂时无法发送，请稍后重试，或导出日志发到客服邮箱"
    );
    s.log("2026-11-07T09:00:00.000000Z  INFO app: later");
    assert_eq!(s.call("previewFeedback", json!({})).await.unwrap(), shown);
    s.submit("导出时卡住了", true).await.unwrap_err();
    // Another choice is another feedback, with the log still the one shown.
    s.submit("导出时卡住了", false).await.unwrap_err();
    s.answer(None);
    s.submit("导出时卡住了", false).await.unwrap();

    {
        let state = s.server.state.lock().unwrap();
        let ids: Vec<_> = state.received.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids[0], ids[1]);
        assert_ne!(ids[1], ids[2]);
        assert_eq!(ids[2], ids[3]);
        assert!(ids.iter().all(|id| id.len() == 32));
        let logs: Vec<_> = state.received.iter().map(|f| f.log.as_deref()).collect();
        assert_eq!(logs, [Some(log.as_str()), Some(log.as_str()), None, None]);
        assert!(
            state
                .received
                .iter()
                .all(|f| f.description == "导出时卡住了")
        );
    }

    // Once it got through, the next feedback shows the log as it is now.
    s.call("previewFeedback", json!({})).await.unwrap();
    let now = s.call("getFeedbackLog", json!({})).await.unwrap();
    assert!(now.as_str().unwrap().ends_with("INFO app: later\n"));
}
