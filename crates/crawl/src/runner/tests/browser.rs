use super::*;
use crate::browser::{BrowserError, RenderedPage};
use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
struct FakeBrowser {
    results: Mutex<VecDeque<Result<RenderedPage, BrowserError>>>,
    /// Where each render opened the page.
    opened: Mutex<Vec<String>>,
    steps: Mutex<Vec<Vec<PageStep>>>,
    entered: Notify,
    release: Notify,
    active: AtomicUsize,
}
impl FakeBrowser {
    fn page(&self, url: &PageUrl, html: &str) {
        self.results.lock().unwrap().push_back(Ok(RenderedPage {
            url: url.as_str().into(),
            html: html.into(),
        }));
    }
}
impl Browser for FakeBrowser {
    fn render<'a>(
        &'a self,
        target: &'a PageTarget,
        steps: &'a [PageStep],
    ) -> Pin<Box<dyn Future<Output = Result<RenderedPage, BrowserError>> + Send + 'a>> {
        Box::pin(async move {
            struct Active<'a>(&'a AtomicUsize);
            impl Drop for Active<'_> {
                fn drop(&mut self) {
                    self.0.fetch_sub(1, Ordering::SeqCst);
                }
            }
            self.active.fetch_add(1, Ordering::SeqCst);
            let _active = Active(&self.active);
            self.opened
                .lock()
                .unwrap()
                .push(target.url().as_str().into());
            self.steps.lock().unwrap().push(steps.to_vec());
            self.entered.notify_one();
            let result = self.results.lock().unwrap().pop_front();
            if let Some(result) = result {
                return result;
            }
            self.release.notified().await;
            self.results.lock().unwrap().pop_front().unwrap()
        })
    }
}

/// The blocks of the text a page found looking for lists is saved with.
fn blocks(text: &str) -> Vec<String> {
    let text: Value = serde_json::from_str(text).unwrap();
    serde_json::from_value(text["blocks"].clone()).unwrap()
}

fn challenge(site: &Site) {
    site.pages.lock().unwrap().insert(
        "/".into(),
        (
            412,
            "<p>HTTP 已读文字</p><script>challenge()</script>".into(),
        ),
    );
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn browser_extracts_people_and_replays_each_list_page(db: SqlitePool) {
    let site = Site::new().await;
    challenge(&site);
    let browser = Arc::new(FakeBrowser::default());
    browser.page(
        &site.url("/"),
        "<title>名单</title><p>张三 a@school.example.edu</p><nav><button onclick='next()'>\
        下一页</button></nav>",
    );
    browser.page(
        &site.url("/"),
        "<title>名单</title><p>李四 b@school.example.edu</p><nav><button onclick='next()'>\
        下一页</button></nav>",
    );
    browser.page(
        &site.url("/"),
        "<title>名单</title><p>王五 c@school.example.edu</p><nav><button disabled>\
        下一页</button></nav>",
    );
    let mut provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    let responder = tokio::spawn(async move {
        while let Some(request) = provider.requests.recv().await {
            let system = request.body["messages"][0]["content"].as_str().unwrap();
            let data = request.body["messages"][1]["content"].as_str().unwrap();
            let output = if system.starts_with("你整理学院") {
                json!({"kind":"list","next":[]})
            } else {
                let (name, email) = if data.contains("张三") {
                    ("张三", "a@school.example.edu")
                } else if data.contains("李四") {
                    ("李四", "b@school.example.edu")
                } else {
                    ("王五", "c@school.example.edu")
                };
                json!({"people":[listed(name, None, vec![email])]})
            };
            assert!(
                request
                    .respond
                    .send(Response::answer(&output.to_string()))
                    .is_ok()
            );
        }
    });
    let id = queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: Some(browser.clone()),
        recognize: super::recognize,
    };
    let net = net::Net::new(vec![]).unwrap();
    let turns = Turns::default();
    for _ in 0..3 {
        let claim = queue::next(&db).await.unwrap().unwrap();
        let outcome = process(
            &db,
            &net,
            &pages,
            &turns,
            &claim,
            &Cooldown::new(Arc::default()),
        )
        .await
        .unwrap();
        assert!(matches!(outcome, Outcome::Done(_)));
        assert!(save(&db, &claim, &outcome).await.unwrap());
    }
    assert!(queue::next(&db).await.unwrap().is_none());
    let records: Vec<(String, String)> =
        sqlx::query_as("SELECT mode,status FROM crawl_page WHERE crawl_id=? ORDER BY id")
            .bind(id)
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(records, vec![("webview".into(), "success".into()); 3]);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM candidate WHERE crawl_id=?")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap(),
        3
    );
    assert_eq!(
        *browser.steps.lock().unwrap(),
        vec![
            vec![],
            vec![PageStep {
                page: Some(2),
                control: "下一页".into()
            }],
            vec![
                PageStep {
                    page: Some(2),
                    control: "下一页".into()
                },
                PageStep {
                    page: Some(3),
                    control: "下一页".into()
                }
            ]
        ]
    );
    responder.abort();
}

/// Reads every page the crawl queues, saving each.
async fn drain(db: &SqlitePool, pages: &Pages) {
    while let Some(claim) = queue::next(db).await.unwrap() {
        finish(db, pages, &claim).await;
    }
}

/// Reads the page claimed, saving it.
async fn finish(db: &SqlitePool, pages: &Pages, claim: &queue::Claim) {
    let outcome = process(
        db,
        &net::Net::new(vec![]).unwrap(),
        pages,
        &Turns::default(),
        claim,
        &Cooldown::new(Arc::default()),
    )
    .await
    .unwrap();
    assert!(matches!(outcome, Outcome::Done(_)));
    assert!(save(db, claim, &outcome).await.unwrap());
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn categories_a_list_loads_when_clicked_are_each_read_in_the_browser(db: SqlitePool) {
    let site = Site::new().await;
    site.pages.lock().unwrap().insert(
        "/".into(),
        (
            200,
            "<title>师资力量</title><ul><li><a>无机化学</a></li><li><a>分析化学</a></li></ul>\
            <ul class='teacher-list'></ul><script src='list.js'></script>"
                .into(),
        ),
    );
    let browser = Arc::new(FakeBrowser::default());
    for (name, email) in [("张三", "a"), ("李四", "b")] {
        browser.page(
            &site.url("/"),
            &format!(
                "<title>师资力量</title><a>无机化学</a><a>分析化学</a>\
                <p>{name} {email}@school.example.edu</p>"
            ),
        );
    }
    let mut provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    let responder = tokio::spawn(async move {
        while let Some(request) = provider.requests.recv().await {
            let system = request.body["messages"][0]["content"].as_str().unwrap();
            let data = request.body["messages"][1]["content"].as_str().unwrap();
            let data: Value = serde_json::from_str(data).unwrap();
            let output = if system.starts_with("你整理学院") {
                // Only the page as it opens offers its controls, after its links.
                if data["links"] == json!(["[无机化学](1)", "[分析化学](2)"]) {
                    json!({"kind": "entry", "next": [1, 2]})
                } else {
                    assert_eq!(data["links"], json!([]));
                    json!({"kind": "list", "next": []})
                }
            } else {
                let text = data.to_string();
                let people: Vec<_> = [
                    ("张三", "a@school.example.edu"),
                    ("李四", "b@school.example.edu"),
                ]
                .into_iter()
                .filter(|(name, _)| text.contains(name))
                .map(|(name, email)| listed(name, None, vec![email]))
                .collect();
                json!({ "people": people })
            };
            assert!(
                request
                    .respond
                    .send(Response::answer(&output.to_string()))
                    .is_ok()
            );
        }
    });
    let id = queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: Some(browser.clone()),
        recognize: super::recognize,
    };
    drain(&db, &pages).await;
    let click = |control: &str| PageStep {
        page: None,
        control: control.into(),
    };
    assert_eq!(
        *browser.steps.lock().unwrap(),
        [vec![click("无机化学")], vec![click("分析化学")]]
    );
    let records: Vec<(String, String, String)> =
        sqlx::query_as("SELECT kind, steps, mode FROM crawl_page WHERE crawl_id = ? ORDER BY id")
            .bind(id)
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(
        records,
        [
            ("entry".into(), "[]".into(), "http".into()),
            (
                "list".into(),
                r#"[{"control":"无机化学"}]"#.into(),
                "webview".into()
            ),
            (
                "list".into(),
                r#"[{"control":"分析化学"}]"#.into(),
                "webview".into()
            ),
        ]
    );
    let names: Vec<String> =
        sqlx::query_scalar("SELECT name FROM candidate WHERE crawl_id = ? ORDER BY name")
            .bind(id)
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(names, ["张三", "李四"]);
    responder.abort();
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_page_reached_over_http_opens_there_in_the_browser(db: SqlitePool) {
    let site = Site::new().await;
    challenge(&site);
    let browser = Arc::new(FakeBrowser::default());
    browser.page(
        &site.url("/"),
        "<title>名单</title><p>张三 a@school.example.edu</p>",
    );
    let mut provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    let responder = tokio::spawn(async move {
        while let Some(request) = provider.requests.recv().await {
            let system = request.body["messages"][0]["content"].as_str().unwrap();
            let output = if system.starts_with("你整理学院") {
                json!({"kind": "list", "next": []})
            } else {
                json!({"people": [listed("张三", None, vec!["a@school.example.edu"])]})
            };
            assert!(
                request
                    .respond
                    .send(Response::answer(&output.to_string()))
                    .is_ok()
            );
        }
    });
    let https = PageUrl::parse(&format!("https://school.example.edu:{}/", site.port)).unwrap();
    let id = queue::start(&db, &[https], model, 1).await.unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: Some(browser.clone()),
        recognize: super::recognize,
    };
    drain(&db, &pages).await;
    assert_eq!(*browser.opened.lock().unwrap(), [site.url("/").as_str()]);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT name FROM candidate WHERE crawl_id = ?")
            .bind(id)
            .fetch_all(&db)
            .await
            .unwrap(),
        ["张三"]
    );
    responder.abort();
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn browser_failures_preserve_http_text_and_preview_still_needs_browser(db: SqlitePool) {
    let site = Site::new().await;
    challenge(&site);
    let provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    let id = queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let mut claim = queue::next(&db).await.unwrap().unwrap();
    let browser = Arc::new(FakeBrowser::default());
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: Some(browser.clone()),
        recognize: super::recognize,
    };
    let mut target = Scope::new(&claim.scope).page(claim.url.clone()).unwrap();
    for (result, reason) in [
        (
            Ok(RenderedPage {
                url: site.url("/").as_str().into(),
                html: include_str!("../../../tests/pages/bupt.html").into(),
            }),
            "验证",
        ),
        (Err(BrowserError::TimedOut), "20 秒"),
        (
            Err(BrowserError::Failed(Failure::new(
                "网站的证书有问题，浏览器无法打开",
            ))),
            "证书",
        ),
        (
            Ok(RenderedPage {
                url: site.url("/").as_str().into(),
                html: "<div id='app'></div><script type='module' src='app.js'></script>".into(),
            }),
            "仍未取得",
        ),
        (
            Ok(RenderedPage {
                url: site.url("/").as_str().into(),
                html: String::new(),
            }),
            "验证",
        ),
        (
            Ok(RenderedPage {
                url: "http://127.0.0.1/private".into(),
                html: "<p>private</p>".into(),
            }),
            "公开",
        ),
        (
            Ok(RenderedPage {
                url: "https://outside.example.org/".into(),
                html: "<p>outside</p>".into(),
            }),
            "允许",
        ),
        (
            Ok(RenderedPage {
                url: site.url("/").as_str().into(),
                html: "x".repeat(crate::fetch::MAX_PAGE_BYTES + 1),
            }),
            "过大",
        ),
    ] {
        browser.results.lock().unwrap().push_back(result);
        let Err(ReadError::Failed(error)) = pages
            .fetch(&db, &claim, &mut target, &Cooldown::new(Arc::default()))
            .await
        else {
            panic!("browser should fail");
        };
        assert!(error.message.contains(reason), "{error:?}");
        assert!(save(&db, &claim, &Outcome::Failed(error)).await.unwrap());
        let (status, mode, text): (String, String, String) =
            sqlx::query_as("SELECT status,mode,text FROM crawl_page WHERE id=?")
                .bind(claim.id)
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!((status.as_str(), mode.as_str()), ("failed", "webview"));
        assert_eq!(blocks(&text), ["HTTP 已读文字"]);
        queue::retry(&db, id, Some(claim.id), 2).await.unwrap();
        claim = queue::next(&db).await.unwrap().unwrap();
    }
    let (m, _) = Pages::local(site.port)
        .fetch(&db, &claim, &mut target, &Cooldown::new(Arc::default()))
        .await
        .unwrap_or_else(|_| panic!("preview HTTP material"));
    assert_eq!(m.needs_browser, Some(crate::BrowserReason::Challenge));
    assert_eq!(m.text(), "HTTP 已读文字");
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn pause_and_cancel_drop_browser_and_reject_late_results(db: SqlitePool) {
    let site = Site::new().await;
    challenge(&site);
    let provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    for action in ["pause", "cancel"] {
        let id = queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
        let claim = queue::next(&db).await.unwrap().unwrap();
        let browser = Arc::new(FakeBrowser::default());
        let pages = Pages {
            writes: db::Writes::default(),
            http: Fetcher::local(site.port),
            browser: Some(browser.clone()),
            recognize: super::recognize,
        };
        let mut target = Scope::new(&claim.scope).page(claim.url.clone()).unwrap();
        let cooldown = Cooldown::new(Arc::default());
        let read = pages.fetch(&db, &claim, &mut target, &cooldown);
        tokio::pin!(read);
        tokio::select! {
            _ = browser.entered.notified() => {},
            _ = &mut read => panic!("browser must remain in flight"),
        }
        queue::control(&db, id, action, 2).await.unwrap();
        tokio::time::pause();
        tokio::time::advance(Duration::from_millis(501)).await;
        // Auto-advance can expire the pool timeout before SQLite's real threads can reply.
        tokio::time::resume();
        assert!(matches!(read.await, Err(ReadError::Gone)));
        assert_eq!(browser.active.load(Ordering::SeqCst), 0);
        let before: (String, Option<String>, String) =
            sqlx::query_as("SELECT status,error,text FROM crawl_page WHERE id=?")
                .bind(claim.id)
                .fetch_one(&db)
                .await
                .unwrap();
        assert!(
            !save(
                &db,
                &claim,
                &Outcome::Failed(Failure::new("迟到的浏览器结果"))
            )
            .await
            .unwrap()
        );
        let after: (String, Option<String>, String) =
            sqlx::query_as("SELECT status,error,text FROM crawl_page WHERE id=?")
                .bind(claim.id)
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(before, after);
        assert_eq!(blocks(&after.2), ["HTTP 已读文字"]);
        if action == "cancel" {
            // Cut short reading it in the browser, it shows as canceled, not as needing a browser.
            let error: Value = serde_json::from_str(&after.1.unwrap()).unwrap();
            assert_eq!(
                (after.0.as_str(), &error["message"]),
                ("failed", &json!("已取消，页面资料尚未整理完成"))
            );
        }
        if action == "pause" {
            queue::control(&db, id, "cancel", 3).await.unwrap();
        }
    }
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn page_failures_and_browser_takeover_log_the_crawl_without_private_content(db: SqlitePool) {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("crawl.log");
    let file = std::fs::File::create(&log_path).unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_ansi(false)
        .with_writer(Mutex::new(file))
        .finish();
    // Two scoped dispatchers avoid tracing-core caching callsites from unsubscribed test threads.
    let _quiet = tracing::subscriber::set_default(tracing::subscriber::NoSubscriber::default());
    let guard = tracing::subscriber::set_default(subscriber);

    let site = Site::new().await;
    let provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    let id = queue::start(&db, &[site.url("/missing")], model, 1)
        .await
        .unwrap();
    let browser = Arc::new(FakeBrowser::default());
    browser
        .results
        .lock()
        .unwrap()
        .push_back(Err(BrowserError::TimedOut));
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: Some(browser),
        recognize: super::recognize,
    };
    let net = net::Net::new(vec![]).unwrap();
    let turns = Turns::default();
    for challenge in [false, true] {
        if challenge {
            site.pages.lock().unwrap().insert(
                "/missing".into(),
                (
                    412,
                    "<p>PRIVATE PAGE 张三 secret@school.example.edu</p><script>challenge()</script>"
                        .into(),
                ),
            );
            queue::retry(&db, id, None, 2).await.unwrap();
        }
        let claim = queue::next(&db).await.unwrap().unwrap();
        let outcome = process(
            &db,
            &net,
            &pages,
            &turns,
            &claim,
            &Cooldown::new(Arc::default()),
        )
        .await
        .unwrap();
        assert!(matches!(outcome, Outcome::Failed(_)));
        save(&db, &claim, &outcome).await.unwrap();
    }
    drop(guard);
    let log = std::fs::read_to_string(log_path).unwrap();
    let marker = format!("crawl{{id={id}}}:");
    for diagnostic in ["page returned HTTP 404", "Challenge", "crawl pages retried"] {
        assert!(
            log.lines()
                .any(|line| line.contains(&marker) && line.contains(diagnostic)),
            "missing crawl-scoped {diagnostic}: {log}"
        );
    }
    for private in [
        "PRIVATE PAGE",
        "张三",
        "secret@school.example.edu",
        "fixture",
    ] {
        assert!(!log.contains(private));
    }
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_profile_the_browser_shows_as_only_a_picture_is_read(db: SqlitePool) {
    let site = Site::new().await;
    site.pages
        .lock()
        .unwrap()
        .insert("/p".into(), (412, "<script>challenge()</script>".into()));
    site.file("/mail.png", "image/png", image(180, 20, "zs@example.edu"));
    let browser = Arc::new(FakeBrowser::default());
    browser.page(&site.url("/p"), "<title>张三</title><img src='/mail.png'>");
    let mut provider = Provider::start().await.unwrap();
    config(&db, &provider.address).await;
    let (_, id) = enrichment::candidate(&db, &site, "/p").await;
    crate::enrich::start(&db, &[id], 2).await.unwrap();
    let claim = queue::next(&db).await.unwrap().unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: Some(browser),
        recognize: super::recognize,
    };
    let net = net::Net::new(vec![]).unwrap();
    let turns = Turns::default();
    let cooldown = Cooldown::new(Arc::default());
    let read = process(&db, &net, &pages, &turns, &claim, &cooldown);
    tokio::pin!(read);
    let request = tokio::select! {
        request = provider.requests.recv() => request.unwrap(),
        result = &mut read => panic!("read without asking the model: {:?}", result.err()),
    };
    assert!(request.body.to_string().contains("zs@example.edu"));
    let answer = targeted("zs@example.edu").to_string();
    request
        .respond
        .send(Response::answer(&answer))
        .ok()
        .unwrap();
    save(&db, &claim, &read.await.unwrap()).await.unwrap();
    let d = review::detail(&db, id).await.unwrap().unwrap();
    assert_eq!(d.candidate.fields.emails, vec!["zs@example.edu"]);
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_list_that_shows_no_teacher_over_http_is_read_in_the_browser(db: SqlitePool) {
    let site = Site::new().await;
    // One list fills in its teachers with scripts; another page is blank until they run.
    site.page(
        "/",
        "<title>教师名单</title><h1>师资队伍</h1><div id='list'></div>",
    );
    site.page("/blank", "<html><body></body></html>");
    let browser = Arc::new(FakeBrowser::default());
    browser.page(
        &site.url("/"),
        "<title>教师名单</title><p>张三 a@school.example.edu</p>",
    );
    browser.page(
        &site.url("/blank"),
        "<title>教师名单</title><p>李四 b@school.example.edu</p>",
    );
    let mut provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    let responder = tokio::spawn(async move {
        while let Some(request) = provider.requests.recv().await {
            let system = request.body["messages"][0]["content"].as_str().unwrap();
            let data = request.body["messages"][1]["content"].as_str().unwrap();
            let output = if system.starts_with("你整理学院") {
                json!({"kind":"list","next":[]})
            } else if data.contains("张三") {
                json!({"people":[listed("张三", None, vec!["a@school.example.edu"])]})
            } else if data.contains("李四") {
                json!({"people":[listed("李四", None, vec!["b@school.example.edu"])]})
            } else {
                json!({"people":[]})
            };
            request
                .respond
                .send(Response::answer(&output.to_string()))
                .ok();
        }
    });
    let id = queue::start(&db, &[site.url("/"), site.url("/blank")], model, 1)
        .await
        .unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: Some(browser),
        recognize: super::recognize,
    };
    let net = net::Net::new(vec![]).unwrap();
    let turns = Turns::default();
    while let Some(claim) = queue::next(&db).await.unwrap() {
        let cooldown = Cooldown::new(Arc::default());
        let outcome = process(&db, &net, &pages, &turns, &claim, &cooldown)
            .await
            .unwrap();
        assert!(save(&db, &claim, &outcome).await.unwrap());
    }
    let records: Vec<(String, String)> =
        sqlx::query_as("SELECT mode,status FROM crawl_page WHERE crawl_id=? ORDER BY id")
            .bind(id)
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(records, vec![("webview".into(), "success".into()); 2]);
    let names: Vec<String> =
        sqlx::query_scalar("SELECT name FROM candidate WHERE crawl_id=? ORDER BY id")
            .bind(id)
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(names, vec!["张三", "李四"]);
    responder.abort();
}

/// Answers routing as `route` does with the request's data, and lists whichever of 张三, 李四 and
/// 王五 a page names.
fn answer(mut provider: Provider, route: impl Fn(&Value) -> Value + Send + 'static) {
    tokio::spawn(async move {
        while let Some(request) = provider.requests.recv().await {
            let system = request.body["messages"][0]["content"].as_str().unwrap();
            let data = request.body["messages"][1]["content"].as_str().unwrap();
            let data: Value = serde_json::from_str(data).unwrap();
            let output = if system.starts_with("你整理学院") {
                route(&data)
            } else {
                let text = data.to_string();
                let people: Vec<_> = (["张三", "李四", "王五"].into_iter())
                    .filter(|name| text.contains(name))
                    .map(|name| listed(name, None, vec![]))
                    .collect();
                json!({ "people": people })
            };
            request
                .respond
                .send(Response::answer(&output.to_string()))
                .ok();
        }
    });
}

/// Answers routing with `kind` and every link offered, and lists as `answer` does; gives the titles
/// of the pages routed.
fn route_all(provider: Provider, kind: &'static str) -> Arc<Mutex<Vec<String>>> {
    let routed = Arc::new(Mutex::new(Vec::new()));
    let titles = routed.clone();
    answer(provider, move |data| {
        (titles.lock().unwrap()).push(data["title"].as_str().unwrap().into());
        let next: Vec<_> = offered(data).into_iter().map(|(_, n)| n).collect();
        json!({"kind": kind, "next": next})
    });
    routed
}

/// Routes the start, titled 首页, as an entry following every link it offers, and the others as
/// `kind` gives by their titles, following none, as the model leaves page turns alone.
fn route_from_start(provider: Provider, kind: fn(&str) -> &'static str) {
    answer(provider, move |data| {
        match data["title"].as_str().unwrap() {
            "首页" => {
                let next: Vec<_> = offered(data).into_iter().map(|(_, n)| n).collect();
                json!({"kind": "entry", "next": next})
            }
            title => json!({"kind": kind(title), "next": []}),
        }
    });
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_list_filled_in_by_scripts_is_read_in_the_browser_though_it_links_others(db: SqlitePool) {
    // As math.nankai.edu.cn lists its teachers in post by a search script, beside links to its
    // other lists; an empty category shows as it did.
    let site = Site::new().await;
    let retired = "<a href='/retired'>退休教师</a>";
    site.page(
        "/",
        &format!("<title>在职教师</title>{retired}<div id='list'></div>"),
    );
    site.page("/assistant", &format!("<title>助教</title>{retired}"));
    site.page("/retired", "<title>退休教师</title><p>李四</p>");
    let browser = Arc::new(FakeBrowser::default());
    browser.page(
        &site.url("/"),
        &format!("<title>在职教师</title>{retired}<p>张三</p>"),
    );
    browser.page(
        &site.url("/assistant"),
        &format!("<title>助教</title>{retired}"),
    );
    let provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    let routed = route_all(provider, "list");
    let starts = [site.url("/"), site.url("/assistant")];
    queue::start(&db, &starts, model, 1).await.unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: Some(browser.clone()),
        recognize: super::recognize,
    };
    drain(&db, &pages).await;
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM candidate ORDER BY name")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(names, ["张三", "李四"]);
    assert_eq!(
        *browser.opened.lock().unwrap(),
        starts.map(|url| url.as_str().to_owned())
    );
    let routed = routed.lock().unwrap();
    assert_eq!(routed.iter().filter(|title| *title == "助教").count(), 1);
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_page_leading_only_a_link_too_far_is_read_in_the_browser(db: SqlitePool) {
    // As econ.fudan.edu.cn's teachers in post, two links from its home page, which over HTTP only
    // offers ways to sort them.
    let site = Site::new().await;
    site.page("/", "<title>首页</title><a href='/szdw'>师资队伍</a>");
    site.page(
        "/szdw",
        "<title>师资队伍</title><a href='/szdw/zzjs'>在职教师</a>",
    );
    site.page(
        "/szdw/zzjs",
        "<title>在职教师</title><ul><li onclick=\"sort('name')\">按拼音排序</li>\
        <li><a href='/szdw/zzjs?sort=title'>按职称排序</a></li></ul><h3>无内容</h3>",
    );
    let browser = Arc::new(FakeBrowser::default());
    browser.page(
        &site.url("/szdw/zzjs"),
        "<title>在职教师</title><p>张三</p>",
    );
    let provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    route_all(provider, "entry");
    queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: Some(browser.clone()),
        recognize: super::recognize,
    };
    drain(&db, &pages).await;
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM candidate")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(names, ["张三"]);
    assert_eq!(*browser.steps.lock().unwrap(), [Vec::<PageStep>::new()]);
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_list_read_again_with_an_address_its_scripts_fill_in_is_read_again(db: SqlitePool) {
    // Over HTTP its address link leads nowhere, which the model takes for no teacher's line.
    let site = Site::new().await;
    site.page("/", "<title>教师名录</title><p>张三 <a>邮箱</a></p>");
    let browser = Arc::new(FakeBrowser::default());
    browser.page(
        &site.url("/"),
        "<title>教师名录</title><p>张三 <a href='mailto:a@school.example.edu'>邮箱</a></p>",
    );
    let mut provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    tokio::spawn(async move {
        while let Some(request) = provider.requests.recv().await {
            let system = request.body["messages"][0]["content"].as_str().unwrap();
            let data = request.body["messages"][1]["content"].as_str().unwrap();
            let output = if system.starts_with("你整理学院") {
                json!({"kind": "list", "next": []})
            } else if data.contains("a@school.example.edu") {
                json!({"people": [listed("张三", None, vec!["a@school.example.edu"])]})
            } else {
                json!({"people": []})
            };
            (request.respond)
                .send(Response::answer(&output.to_string()))
                .ok();
        }
    });
    queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: Some(browser.clone()),
        recognize: super::recognize,
    };
    drain(&db, &pages).await;
    let emails: Vec<String> = sqlx::query_scalar("SELECT emails FROM candidate")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(emails, [r#"["a@school.example.edu"]"#]);
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_page_the_browser_finds_moved_to_its_link_does_not_lead_on_to_itself(db: SqlitePool) {
    // Two links from the start, its scripts take it to the address it links.
    let site = Site::new().await;
    site.page("/", "<title>首页</title><a href='/szdw'>师资队伍</a>");
    site.page(
        "/szdw",
        "<title>师资队伍</title><a href='/catalog'>教师名录</a>",
    );
    let catalog = "<title>教师名录</title><a href='/canonical'>教师名录</a><h3>无内容</h3>";
    site.page("/catalog", catalog);
    let browser = Arc::new(FakeBrowser::default());
    browser.page(&site.url("/canonical"), catalog);
    let provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    route_all(provider, "entry");
    queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: Some(browser.clone()),
        recognize: super::recognize,
    };
    drain(&db, &pages).await;
    assert_eq!(
        *browser.opened.lock().unwrap(),
        [site.url("/catalog").as_str()]
    );
    let limits: String = sqlx::query_scalar("SELECT limits FROM crawl")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(limits, "[]");
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_category_showing_nothing_new_when_clicked_is_not_read_again(db: SqlitePool) {
    // As math.nankai.edu.cn's filter groups, which only unfold their options when clicked, beside
    // its list paged in the browser: the group's next page shows nothing new either.
    let site = Site::new().await;
    let list = |name: &str, next: &str| {
        format!(
            "<title>在职教师</title><ul><li onclick=\"unfold('dept')\">部门</li></ul>\
            <p>{name}</p><nav><button {next}>下一页</button></nav>"
        )
    };
    let more = "onclick='next()'";
    site.page("/", &list("张三", more));
    let browser = Arc::new(FakeBrowser::default());
    // As many as reading on past the group's second page would take.
    for (name, next) in [
        ("张三", more),
        ("张三", more),
        ("李四", more),
        ("李四", more),
        ("王五", "disabled"),
        ("王五", "disabled"),
    ] {
        browser.page(&site.url("/"), &list(name, next));
    }
    let provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    let routed = route_all(provider, "list");
    queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: Some(browser.clone()),
        recognize: super::recognize,
    };
    drain(&db, &pages).await;
    let step = |control: &str, page| PageStep {
        page,
        control: control.into(),
    };
    let (group, next) = (step("部门", None), |page| step("下一页", Some(page)));
    assert_eq!(
        *browser.steps.lock().unwrap(),
        [
            vec![],
            vec![group.clone()],
            vec![next(2)],
            vec![group, next(2)],
            vec![next(2), next(3)]
        ]
    );
    // The model routes the list's first page only: the group shows nothing new, and the pages
    // turned to are the list's own.
    assert_eq!(*routed.lock().unwrap(), ["在职教师"]);
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_list_at_another_address_is_read_once(db: SqlitePool) {
    // As math.nankai.edu.cn's teachers in post at /33156/list.htm and /_t1017/33156/list.htm.
    let site = Site::new().await;
    site.page(
        "/",
        "<title>首页</title><a href='/szdw'>在职教师</a><a href='/t1/szdw'>教师</a>",
    );
    let list = "<title>在职教师</title><p>张三</p>";
    site.page("/szdw", list);
    site.page("/t1/szdw", list);
    let browser = Arc::new(FakeBrowser::default());
    (browser.results.lock().unwrap()).push_back(Err(BrowserError::TimedOut));
    let provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    let routed = route_all(provider, "entry");
    queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: Some(browser.clone()),
        recognize: super::recognize,
    };
    drain(&db, &pages).await;
    assert_eq!(*routed.lock().unwrap(), ["首页", "在职教师"]);
    // It gives nothing to read in the browser either: the one read before showed whether to.
    assert!(browser.opened.lock().unwrap().is_empty());
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_list_a_link_too_far_at_another_address_is_read_once_though_its_menu_links_elsewhere(
    db: SqlitePool,
) {
    // As math.nankai.edu.cn's teachers in post at /33156/list.htm, linking all of them at
    // /_t1017/33156/list.htm, whose menu links each page at /_s91/….psp.
    let site = Site::new().await;
    site.page("/", "<title>首页</title><a href='/szdw'>在职教师</a>");
    let list = |retired| {
        format!(
            "<title>在职教师</title><a href='{retired}'>退休教师</a>\
            <a href='/t1/szdw'>全体教师</a><p>张三</p>"
        )
    };
    site.page("/szdw", &list("/retired"));
    site.page("/t1/szdw", &list("/s1/retired.psp"));
    site.page("/retired", "<title>退休教师</title><p>李四</p>");
    let provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    let routed = route_all(provider, "list");
    queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: None,
        recognize: super::recognize,
    };
    drain(&db, &pages).await;
    assert_eq!(*routed.lock().unwrap(), ["首页", "在职教师", "退休教师"]);
}

#[tokio::test]
async fn a_list_a_link_too_far_linking_its_teacher_s_page_elsewhere_is_read() {
    // Its teachers' pages are read from it, a link too far or not, whether the list read before
    // linked another page of theirs or none, linking back to itself as it is written or with a
    // slash more, or showing as text what this one writes for a link; and whether the link names
    // them or not, as a card's 个人主页 under their name, or names them spaced out.
    let cards = [
        ("<p><a href='PROFILE'>张三</a></p>", "张三"),
        (
            "<article><p>张三</p><p><a href='PROFILE'>个人主页</a></p></article>",
            "个人主页",
        ),
        ("<p><a href='PROFILE'>张 三</a></p>", "张 三"),
    ];
    let lists: [(&str, &[&str]); 3] = [
        ("/old", &["/new", "/old"]),
        ("/szdw", &["/new"]),
        ("/szdw/", &["/new"]),
    ];
    let mut cases = Vec::new();
    for (card, label) in cards {
        for (old, read) in lists {
            cases.push((card.replace("PROFILE", old), card, label, read));
        }
    }
    cases.push(("<p>[张三](2)</p>".to_owned(), cards[0].0, "张三", &["/new"]));
    for (first, card, label, read) in cases {
        let (dir, db) = test_fixtures::db::open().await.unwrap();
        let site = Site::new().await;
        site.page("/", "<title>首页</title><a href='/szdw'>在职教师</a>");
        let list = |teacher: &str| {
            format!("<title>在职教师</title><a href='/t1/szdw'>全体教师</a>{teacher}")
        };
        site.page("/szdw", &list(&first));
        site.page("/t1/szdw", &list(&card.replace("PROFILE", "/new")));
        for path in ["/old", "/new"] {
            site.page(path, "<title>张三</title><p>张三 a@school.example.edu</p>");
        }
        let mut provider = Provider::start().await.unwrap();
        let model = config(&db, &provider.address).await;
        tokio::spawn(async move {
            while let Some(request) = provider.requests.recv().await {
                let system = request.body["messages"][0]["content"].as_str().unwrap();
                let data = request.body["messages"][1]["content"].as_str().unwrap();
                let data: Value = serde_json::from_str(data).unwrap();
                let output = if system.starts_with("你整理学院") {
                    let next: Vec<_> = (offered(&data).into_iter())
                        .filter(|(text, _)| ["在职教师", "全体教师"].contains(&text.as_str()))
                        .map(|(_, n)| n)
                        .collect();
                    let kind = if data["title"] == "首页" {
                        "entry"
                    } else {
                        "list"
                    };
                    json!({"kind": kind, "next": next})
                } else if system.starts_with("你从教师") {
                    let text = data.to_string();
                    let link = (text.split(&format!("[{label}](")).nth(1))
                        .and_then(|rest| rest.split(')').next()?.parse::<usize>().ok());
                    json!({"people": [listed("张三", link, vec![])]})
                } else {
                    super::targeted("a@school.example.edu")
                };
                request
                    .respond
                    .send(Response::answer(&output.to_string()))
                    .ok();
            }
        });
        queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
        let pages = Pages {
            writes: db::Writes::default(),
            http: Fetcher::local(site.port),
            browser: None,
            recognize: super::recognize,
        };
        drain(&db, &pages).await;
        let urls: Vec<String> = sqlx::query_scalar(
            "SELECT url FROM crawl_page WHERE target_id IS NOT NULL ORDER BY url",
        )
        .fetch_all(&db)
        .await
        .unwrap();
        let read: Vec<_> = read
            .iter()
            .map(|p| site.url(p).as_str().to_owned())
            .collect();
        assert_eq!(urls, read, "{first}");
        db.close().await;
        dir.close().unwrap();
    }
}

#[tokio::test]
async fn a_list_a_link_too_far_reading_as_a_page_read_as_no_list_is_read() {
    // The page read before linked its teacher off the site, which routing does not show, so that
    // it read as no list; this one links them on the site.
    let (dir, db) = test_fixtures::db::open().await.unwrap();
    let site = Site::new().await;
    site.page(
        "/",
        "<title>首页</title><a href='/a'>信息</a><a href='/nav'>导航</a>",
    );
    site.page("/nav", "<title>导航</title><a href='/b'>信息</a>");
    site.page(
        "/a",
        "<title>信息</title><p><a href='https://outside.example.org/zhang'>张三</a></p>",
    );
    site.page("/b", "<title>信息</title><p><a href='/p'>张三</a></p>");
    site.page("/p", "<title>张三</title><p>张三 a@school.example.edu</p>");
    let mut provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    tokio::spawn(async move {
        while let Some(request) = provider.requests.recv().await {
            let system = request.body["messages"][0]["content"].as_str().unwrap();
            let data = request.body["messages"][1]["content"].as_str().unwrap();
            let data: Value = serde_json::from_str(data).unwrap();
            let output = if system.starts_with("你整理学院") {
                let offered = offered(&data);
                let kind = match data["title"].as_str() {
                    Some("首页" | "导航") => "entry",
                    _ if offered.is_empty() => "other",
                    _ => "list",
                };
                let next: Vec<_> = (offered.into_iter())
                    .filter(|(text, _)| text != "张三")
                    .map(|(_, n)| n)
                    .collect();
                json!({"kind": kind, "next": next})
            } else if system.starts_with("你从教师") {
                json!({"people": [listed("张三", Some(1), vec![])]})
            } else {
                super::targeted("a@school.example.edu")
            };
            request
                .respond
                .send(Response::answer(&output.to_string()))
                .ok();
        }
    });
    queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: None,
        recognize: super::recognize,
    };
    drain(&db, &pages).await;
    let urls: Vec<String> =
        sqlx::query_scalar("SELECT url FROM crawl_page WHERE target_id IS NOT NULL")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(urls, [site.url("/p").as_str()]);
    db.close().await;
    dir.close().unwrap();
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_page_reading_as_another_but_leading_elsewhere_is_read(db: SqlitePool) {
    let site = Site::new().await;
    site.page(
        "/",
        "<title>首页</title><a href='/a'>化学系</a><a href='/b'>物理系</a>",
    );
    for dept in ["a", "b"] {
        site.page(
            &format!("/{dept}"),
            &format!("<title>系</title><a href='/{dept}/szdw'>师资队伍</a>"),
        );
    }
    site.page("/a/szdw", "<title>师资队伍</title><p>张三</p>");
    site.page("/b/szdw", "<title>师资队伍</title><p>李四</p>");
    let provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    route_all(provider, "entry");
    queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: None,
        recognize: super::recognize,
    };
    drain(&db, &pages).await;
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM candidate ORDER BY name")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(names, ["张三", "李四"]);
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_list_reading_as_another_but_for_the_addresses_its_links_give_is_read(db: SqlitePool) {
    // Two teachers named 张三, each list linking its one's address.
    let site = Site::new().await;
    site.page(
        "/",
        "<title>首页</title><a href='/a'>化学系</a><a href='/b'>物理系</a>",
    );
    for dept in ["a", "b"] {
        site.page(
            &format!("/{dept}"),
            &format!(
                "<title>教师名录</title>\
                <p>张三 <a href='mailto:{dept}@school.example.edu'>邮箱</a></p>"
            ),
        );
    }
    let mut provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    tokio::spawn(async move {
        while let Some(request) = provider.requests.recv().await {
            let system = request.body["messages"][0]["content"].as_str().unwrap();
            let data = request.body["messages"][1]["content"].as_str().unwrap();
            let output = if system.starts_with("你整理学院") {
                let data: Value = serde_json::from_str(data).unwrap();
                let next: Vec<_> = offered(&data).into_iter().map(|(_, n)| n).collect();
                json!({"kind": "entry", "next": next})
            } else {
                let people: Vec<_> = (["a@school.example.edu", "b@school.example.edu"].into_iter())
                    .filter(|address| data.contains(address))
                    .map(|address| listed("张三", None, vec![address]))
                    .collect();
                json!({ "people": people })
            };
            (request.respond)
                .send(Response::answer(&output.to_string()))
                .ok();
        }
    });
    queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: None,
        recognize: super::recognize,
    };
    drain(&db, &pages).await;
    let emails: Vec<String> = sqlx::query_scalar("SELECT emails FROM candidate ORDER BY emails")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(
        emails,
        [r#"["a@school.example.edu"]"#, r#"["b@school.example.edu"]"#]
    );
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_page_read_before_a_link_too_far_is_read_again_nearer_the_start(db: SqlitePool) {
    // Its copy two links from the start was read while it was, so did not follow its link.
    let site = Site::new().await;
    site.page(
        "/",
        "<title>首页</title><a href='/near'>师资</a><a href='/dept'>学院</a>",
    );
    let entry = "<title>入口</title><a href='/teachers'>教师名录</a>";
    site.page("/near", entry);
    site.page("/dept", "<title>学院</title><a href='/far'>师资</a>");
    site.page("/far", entry);
    site.page("/teachers", "<title>教师名录</title><p>张三</p>");
    let provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    route_all(provider, "entry");
    queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: None,
        recognize: super::recognize,
    };
    let start = queue::next(&db).await.unwrap().unwrap();
    finish(&db, &pages, &start).await;
    let near = queue::next(&db).await.unwrap().unwrap();
    assert_eq!(near.url, site.url("/near"));
    drain(&db, &pages).await;
    finish(&db, &pages, &near).await;
    drain(&db, &pages).await;
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM candidate")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(names, ["张三"]);
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_page_reading_as_another_but_offering_a_category_to_click_is_read(db: SqlitePool) {
    let site = Site::new().await;
    site.page(
        "/",
        "<title>首页</title><a href='/a'>师资</a><a href='/b'>教师</a>",
    );
    site.page(
        "/a",
        "<title>教师名录</title><a href='/'>首页</a><ul><li>教授</li></ul>",
    );
    let category = "<title>教师名录</title><a href='/'>首页</a>\
        <ul><li onclick=\"show('prof')\">教授</li></ul>";
    site.page("/b", category);
    let browser = Arc::new(FakeBrowser::default());
    browser.page(&site.url("/b"), &format!("{category}<p>张三</p>"));
    let provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    route_all(provider, "entry");
    queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: Some(browser.clone()),
        recognize: super::recognize,
    };
    drain(&db, &pages).await;
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM candidate")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(names, ["张三"]);
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_list_reading_as_another_but_paging_on_is_read(db: SqlitePool) {
    // The two link the same next page, which the first marks as no page to turn to.
    let site = Site::new().await;
    site.page(
        "/",
        "<title>首页</title><a href='/a'>在职教师</a><a href='/b'>教师名录</a>",
    );
    for (path, class) in [("/a", " class='disabled'"), ("/b", "")] {
        site.page(
            path,
            &format!("<title>名单</title><p>张三</p><a href='/p2'{class}>下一页</a>"),
        );
    }
    site.page("/p2", "<title>名单</title><p>李四</p>");
    let provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    route_from_start(provider, |_| "list");
    queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: None,
        recognize: super::recognize,
    };
    drain(&db, &pages).await;
    let names: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT name FROM candidate ORDER BY name")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(names, ["张三", "李四"]);
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_page_reading_as_another_but_for_its_title_is_read(db: SqlitePool) {
    // News naming a teacher, and his department's list.
    let site = Site::new().await;
    site.page(
        "/",
        "<title>首页</title><a href='/news'>新闻</a><a href='/list'>师资</a>",
    );
    site.page("/news", "<title>新闻动态</title><p>张三</p>");
    site.page("/list", "<title>教师名单</title><p>张三</p>");
    let provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    route_from_start(provider, |title| match title {
        "新闻动态" => "other",
        _ => "list",
    });
    queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: None,
        recognize: super::recognize,
    };
    drain(&db, &pages).await;
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM candidate")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(names, ["张三"]);
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_page_reading_as_another_but_for_where_an_address_sits_is_read(db: SqlitePool) {
    // One gives the address in its menu, which is no one's in particular, the other below the name.
    let site = Site::new().await;
    site.page(
        "/",
        "<title>首页</title><a href='/a'>名录</a><a href='/b'>名单</a>",
    );
    let list = |part| {
        format!("<title>教师名录</title><p>张三</p><{part}><p>a@school.example.edu</p></{part}>")
    };
    site.page("/a", &list("nav"));
    site.page("/b", &list("div"));
    let mut provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    tokio::spawn(async move {
        while let Some(request) = provider.requests.recv().await {
            let system = request.body["messages"][0]["content"].as_str().unwrap();
            let data = request.body["messages"][1]["content"].as_str().unwrap();
            let data: Value = serde_json::from_str(data).unwrap();
            let output = if !system.starts_with("你整理学院") {
                json!({"people": [{"n": "张三", "e": ["a@school.example.edu"]}]})
            } else if data["title"] == "首页" {
                let next: Vec<_> = offered(&data).into_iter().map(|(_, n)| n).collect();
                json!({"kind": "entry", "next": next})
            } else {
                json!({"kind": "list", "next": []})
            };
            request
                .respond
                .send(Response::answer(&output.to_string()))
                .ok();
        }
    });
    queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: None,
        recognize: super::recognize,
    };
    drain(&db, &pages).await;
    let emails: Vec<String> =
        sqlx::query_scalar("SELECT emails FROM candidate WHERE emails != '[]' ORDER BY id")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(emails, [r#"["a@school.example.edu"]"#]);
}

/// Answers routing a page writing 研究方向 as a profile, any other naming 张三 or 李四 as a list and
/// the rest as an entry, either following all it offers; reads a profile as 张三's, and lists the
/// two as `answer` does.
fn route_profiles(mut provider: Provider) {
    tokio::spawn(async move {
        while let Some(request) = provider.requests.recv().await {
            let system = request.body["messages"][0]["content"].as_str().unwrap();
            let data = request.body["messages"][1]["content"].as_str().unwrap();
            let data: Value = serde_json::from_str(data).unwrap();
            let output = if system.starts_with("你整理一位导师") {
                super::person("张三", "a@school.example.edu")
            } else if !system.starts_with("你整理学院") {
                let text = data.to_string();
                let people: Vec<_> = (["张三", "李四"].into_iter())
                    .filter(|name| text.contains(name))
                    .map(|name| listed(name, None, vec![]))
                    .collect();
                json!({ "people": people })
            } else if data.to_string().contains("研究方向") {
                json!({"kind": "profile", "next": []})
            } else {
                let text = data.to_string();
                let named = text.contains("张三") || text.contains("李四");
                let next: Vec<_> = offered(&data).into_iter().map(|(_, n)| n).collect();
                json!({"kind": if named { "list" } else { "entry" }, "next": next})
            };
            request
                .respond
                .send(Response::answer(&output.to_string()))
                .ok();
        }
    });
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_category_showing_one_teacher_s_page_is_not_where_their_page_is(db: SqlitePool) {
    // As scst.suda.edu.cn's teachers under one initial: the address is the whole list's.
    let site = Site::new().await;
    let menu = "<title>教师名录</title><ul><li onclick=\"show('phys')\">物理系</li></ul>";
    site.page("/", &format!("{menu}<p>李四</p>"));
    let browser = Arc::new(FakeBrowser::default());
    browser.page(
        &site.url("/"),
        &format!("{menu}<p>张三</p><p>研究方向：机器学习</p><p>a@school.example.edu</p>"),
    );
    let provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    route_profiles(provider);
    queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: Some(browser.clone()),
        recognize: super::recognize,
    };
    drain(&db, &pages).await;
    let read: Vec<(String, Option<String>, String)> =
        sqlx::query_as("SELECT name, profile_url, homepages FROM candidate ORDER BY name")
            .fetch_all(&db)
            .await
            .unwrap();
    let none = |name: &str| (name.into(), None, "[]".into());
    assert_eq!(read, [none("张三"), none("李四")]);
    let profiles: i64 = sqlx::query_scalar("SELECT count(*) FROM candidate_profile")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(profiles, 0);
}

#[tokio::test]
async fn a_list_naming_its_teachers_leaves_its_index_of_initials_alone() {
    // Each initial shows some of them, as scst.suda.edu.cn's 26 letters do; with no one named, the
    // index is the way to them.
    for (named, clicked) in [("<p>张三</p><p>李四</p>", 0), ("", 2)] {
        let (dir, db) = test_fixtures::db::open().await.unwrap();
        let site = Site::new().await;
        let index = "<a href='javascript:void(0)' onclick=\"show('a')\">A</a>\
            <a href='javascript:void(0)' onclick=\"show('b')\">B</a>";
        site.page(
            "/",
            &format!("<title>教师名录</title>{named}<div>{index}</div>"),
        );
        let browser = Arc::new(FakeBrowser::default());
        for name in ["张三", "李四"] {
            browser.page(
                &site.url("/"),
                &format!("<title>教师名录</title><p>{name}</p><div>{index}</div>"),
            );
        }
        let provider = Provider::start().await.unwrap();
        let model = config(&db, &provider.address).await;
        route_profiles(provider);
        queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
        let pages = Pages {
            writes: db::Writes::default(),
            http: Fetcher::local(site.port),
            browser: Some(browser.clone()),
            recognize: super::recognize,
        };
        drain(&db, &pages).await;
        let names: Vec<String> = sqlx::query_scalar("SELECT name FROM candidate ORDER BY name")
            .fetch_all(&db)
            .await
            .unwrap();
        assert_eq!(names, ["张三", "李四"], "{named}");
        assert_eq!(browser.opened.lock().unwrap().len(), clicked, "{named}");
        db.close().await;
        dir.close().unwrap();
    }
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_category_whose_first_page_reads_as_the_list_s_pages_on(db: SqlitePool) {
    // Its teachers come first in the whole list too, which goes on to another department's.
    let site = Site::new().await;
    let list = |name: &str, next: &str| {
        format!(
            "<title>名单</title><ul><li onclick=\"dept('phys')\">物理系</li></ul>\
            <p>{name}</p><nav><button {next}>下一页</button></nav>"
        )
    };
    let more = "onclick='next()'";
    site.page("/", &list("张三", more));
    let browser = Arc::new(FakeBrowser::default());
    for (name, next) in [
        ("张三", more),
        ("张三", more),
        ("李四", "disabled"),
        ("王五", "disabled"),
    ] {
        browser.page(&site.url("/"), &list(name, next));
    }
    let provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    route_all(provider, "list");
    queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: Some(browser.clone()),
        recognize: super::recognize,
    };
    drain(&db, &pages).await;
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM candidate ORDER BY name")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(names, ["张三", "李四", "王五"]);
}

#[tokio::test]
async fn a_page_turn_showing_the_page_it_turned_from_fails() {
    // Its pager still says 1/3, as when the click did nothing; counting no pages, it ends them, as
    // a carousel's next does.
    for (count, failed) in [("<span>1/3</span>", 1), ("", 0)] {
        let (dir, db) = test_fixtures::db::open().await.unwrap();
        let site = Site::new().await;
        let list = format!(
            "<title>名单</title><p>张三</p>\
            <div class='pager'>{count}<button onclick='next()'>下一页</button></div>"
        );
        site.page("/", &list);
        let browser = Arc::new(FakeBrowser::default());
        for _ in 0..2 {
            browser.page(&site.url("/"), &list);
        }
        let provider = Provider::start().await.unwrap();
        let model = config(&db, &provider.address).await;
        route_all(provider, "list");
        queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
        let pages = Pages {
            writes: db::Writes::default(),
            http: Fetcher::local(site.port),
            browser: Some(browser.clone()),
            recognize: super::recognize,
        };
        drain(&db, &pages).await;
        let failures: Vec<String> = sqlx::query_scalar(
            "SELECT json_extract(error, '$.message') FROM crawl_page WHERE status = 'failed'",
        )
        .fetch_all(&db)
        .await
        .unwrap();
        assert_eq!(failures.len(), failed, "{count}");
        assert_eq!(browser.opened.lock().unwrap().len(), 2, "{count}");
        db.close().await;
        dir.close().unwrap();
    }
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_list_reading_over_http_as_one_its_scripts_left_alone_is_read_in_the_browser(
    db: SqlitePool,
) {
    // Both show no teacher over HTTP and link one department; the second's scripts link another.
    let site = Site::new().await;
    site.page(
        "/",
        "<title>首页</title><p>王五</p><a href='/a'>在职教师</a><a href='/b'>兼职教师</a>",
    );
    let list = "<title>名单</title><a href='/dept-a'>化学系</a><div id='list'></div>";
    site.page("/a", list);
    site.page("/b", list);
    site.page("/dept-a", "<title>化学系</title><p>张三</p>");
    site.page("/dept-b", "<title>物理系</title><p>李四</p>");
    let browser = Arc::new(FakeBrowser::default());
    browser.page(&site.url("/a"), list);
    browser.page(
        &site.url("/b"),
        "<title>名单</title><a href='/dept-b'>物理系</a>",
    );
    let provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    route_all(provider, "list");
    queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: Some(browser.clone()),
        recognize: super::recognize,
    };
    drain(&db, &pages).await;
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM candidate ORDER BY name")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(names, ["张三", "李四", "王五"]);
    assert_eq!(
        *browser.opened.lock().unwrap(),
        ["/a", "/b"].map(|path| site.url(path).as_str().to_owned())
    );
}

#[tokio::test]
async fn a_list_paging_on_only_by_a_jump_notes_its_later_pages_unread() {
    // Its pager counts the pages and only jumps on, to the page written in its box: counting two;
    // writing the page it is on and how many there are apart; on the second of five, going back
    // by 上一页 or by a link naming the page it leads to; at an address naming no page, going back
    // by such a link on the third, or on the fourth, as the pager the link is in counts, though
    // another pager before it counts otherwise.
    for (start, pagers) in [
        ("/", vec![("/", "<span>1/2</span>")]),
        ("/", vec![("/", "<span>第 1 页</span><span>共 5 页</span>")]),
        (
            "/",
            vec![(
                "/",
                "<span class='current_page'>1</span><span>共 5 页</span>",
            )],
        ),
        (
            "/?page=2",
            vec![
                ("/?page=2", "<a href='/?page=1'>上一页</a><span>2/5</span>"),
                ("/?page=1", "<span>1/5</span><a href='/?page=2'>下一页</a>"),
            ],
        ),
        (
            "/?page=2",
            vec![
                ("/?page=2", "<a href='/?page=1'>返回</a><span>2/5</span>"),
                ("/?page=1", "<span>1/5</span><a href='/?page=2'>下一页</a>"),
            ],
        ),
        (
            "/list3.htm",
            vec![
                (
                    "/list3.htm",
                    "<a href='/list.htm?page=2'>返回</a><span>3/5</span>",
                ),
                (
                    "/list.htm?page=2",
                    "<span>2/5</span><a href='/list3.htm'>下一页</a>",
                ),
            ],
        ),
        (
            "/list4.htm",
            vec![
                (
                    "/list4.htm",
                    "<span>2/5</span><a href='javascript:void(0)'>上一页</a></div>\
                    <div class='pager'><span>4/6</span><a href='/list.htm?page=3'>返回</a>",
                ),
                (
                    "/list.htm?page=3",
                    "<span>3/6</span><a href='/list4.htm'>下一页</a>",
                ),
            ],
        ),
    ] {
        let (dir, db) = test_fixtures::db::open().await.unwrap();
        let site = Site::new().await;
        for (path, pager) in &pagers {
            site.page(
                path,
                &format!(
                    "<title>名单</title><p>张三</p><div class='pager'>{pager}\
                    <input type='text' value='3'><button onclick='go()'>跳转</button></div>"
                ),
            );
        }
        let provider = Provider::start().await.unwrap();
        let model = config(&db, &provider.address).await;
        route_all(provider, "list");
        queue::start(&db, &[site.url(start)], model, 1)
            .await
            .unwrap();
        let pages = Pages {
            writes: db::Writes::default(),
            http: Fetcher::local(site.port),
            browser: None,
            recognize: super::recognize,
        };
        drain(&db, &pages).await;
        let read: i64 =
            sqlx::query_scalar("SELECT count(*) FROM crawl_page WHERE status = 'success'")
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(read, i64::try_from(pagers.len()).unwrap(), "{pagers:?}");
        let limits: String = sqlx::query_scalar("SELECT limits FROM crawl")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(
            limits, r#"["有的名单只能输入页码跳转，后面的页没有读"]"#,
            "{pagers:?}"
        );
        db.close().await;
        dir.close().unwrap();
    }
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_list_read_again_as_it_was_but_for_controls_is_routed_again(db: SqlitePool) {
    // Its scripts make the category written over HTTP one to click.
    let site = Site::new().await;
    site.page("/", "<title>教师名录</title><ul><li>教授</li></ul>");
    let browser = Arc::new(FakeBrowser::default());
    let category = "<title>教师名录</title><ul><li onclick=\"show('prof')\">教授</li></ul>";
    browser.page(&site.url("/"), category);
    browser.page(&site.url("/"), &format!("{category}<p>张三</p>"));
    let provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    route_all(provider, "list");
    queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: Some(browser.clone()),
        recognize: super::recognize,
    };
    drain(&db, &pages).await;
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM candidate")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(names, ["张三"]);
    let clicked = PageStep {
        page: None,
        control: "教授".into(),
    };
    assert_eq!(*browser.steps.lock().unwrap(), [vec![], vec![clicked]]);
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_list_the_browser_fails_to_read_again_still_leads_to_the_others(db: SqlitePool) {
    let site = Site::new().await;
    site.page(
        "/",
        "<title>在职教师</title><a href='/retired'>退休教师</a><div id='list'></div>",
    );
    site.page("/retired", "<title>退休教师</title><p>李四</p>");
    let browser = Arc::new(FakeBrowser::default());
    (browser.results.lock().unwrap()).push_back(Err(BrowserError::TimedOut));
    let provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    route_all(provider, "list");
    queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: Some(browser.clone()),
        recognize: super::recognize,
    };
    drain(&db, &pages).await;
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM candidate")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(names, ["李四"]);
    // It shows as failed, to retry.
    let pages: Vec<(String, String)> =
        sqlx::query_as("SELECT url, status FROM crawl_page ORDER BY id")
            .fetch_all(&db)
            .await
            .unwrap();
    let page = |path, status: &str| (site.url(path).as_str().to_owned(), status.to_owned());
    assert_eq!(pages, [page("/", "failed"), page("/retired", "success")]);
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_profile_that_shows_no_teacher_over_http_is_read_in_the_browser(db: SqlitePool) {
    let site = Site::new().await;
    site.page(
        "/",
        "<title>教师名单</title><p><a href='/teacher?uid=1'>张三</a></p>",
    );
    // As cta.jxufe.edu.cn's teacher pages are over HTTP, before their scripts fill them in.
    site.page(
        "/teacher?uid=1",
        "<title>教师主页</title><h1>{{name}}</h1><p>邮箱 {{email}}</p><script src='/app.js'></script>",
    );
    let browser = Arc::new(FakeBrowser::default());
    browser.page(
        &site.url("/teacher?uid=1"),
        "<title>教师主页</title><h1>张三</h1><p>邮箱 a@school.example.edu</p>",
    );
    let mut provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    let responder = tokio::spawn(async move {
        while let Some(request) = provider.requests.recv().await {
            let system = request.body["messages"][0]["content"].as_str().unwrap();
            let output = if system.starts_with("你整理学院") {
                json!({"kind":"list","next":[]})
            } else if system.starts_with("你从教师") {
                json!({"people":[listed("张三", Some(1), vec![])]})
            } else {
                targeted("a@school.example.edu")
            };
            request
                .respond
                .send(Response::answer(&output.to_string()))
                .ok();
        }
    });
    let id = queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: Some(browser),
        recognize: super::recognize,
    };
    drain(&db, &pages).await;
    let profile: (String, String) =
        sqlx::query_as("SELECT mode, status FROM crawl_page WHERE crawl_id=? AND kind='profile'")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(profile, ("webview".into(), "success".into()));
    let emails: Vec<String> = sqlx::query_scalar("SELECT value FROM candidate, json_each(emails)")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(emails, ["a@school.example.edu"]);
    responder.abort();
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_list_stops_at_a_page_turned_to_finding_no_one_new(db: SqlitePool) {
    // As faculty-civileng.tongji.edu.cn's pages, each showing everyone above the ten it turns to;
    // a category whose first page shows only those found before still turns once.
    let site = Site::new().await;
    let next = |page: &str| format!("<a href='{page}'>下一页</a>");
    site.page(
        "/",
        &format!(
            "<title>教师名单</title><p>张三</p>{}<a href='/dept'>按系</a>",
            next("/?page=2")
        ),
    );
    site.page(
        "/?page=2",
        &format!("<title>教师名单</title><p>李四</p>{}", next("/?page=3")),
    );
    site.page(
        "/?page=3",
        &format!("<title>教师名单</title><p>张三</p>{}", next("/?page=4")),
    );
    site.page("/?page=4", "<title>教师名单</title><p>王五</p>");
    site.page(
        "/dept",
        &format!("<title>按系</title><p>张三</p>{}", next("/dept?page=2")),
    );
    site.page(
        "/dept?page=2",
        &format!("<title>按系</title><p>张三</p>{}", next("/dept?page=3")),
    );
    site.page("/dept?page=3", "<title>按系</title><p>王五</p>");
    let provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    route_all(provider, "list");
    queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: None,
        recognize: super::recognize,
    };
    drain(&db, &pages).await;
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM candidate ORDER BY id")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(names, ["张三", "李四"]);
    let visits = site.visits.lock().unwrap().clone();
    for read in ["/?page=3", "/dept?page=2"] {
        assert!(visits.contains(&read.to_owned()), "{read}");
    }
    for unread in ["/?page=4", "/dept?page=3"] {
        assert!(!visits.contains(&unread.to_owned()), "{unread}");
    }
    let limits: String = sqlx::query_scalar("SELECT limits FROM crawl")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(
        limits,
        r#"["有的名单翻到的一页没有新导师，后面的页没有读"]"#
    );
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_list_pages_on_past_the_limit_of_pages_finding_no_one_new(db: SqlitePool) {
    // The crawl has read as many pages finding no teacher new to it as it reads, as a list misread
    // might give: a list finding someone new still pages on, and nothing else is read.
    let site = Site::new().await;
    site.page(
        "/",
        "<title>教师名单</title><p>张三</p><a href='/?page=2'>下一页</a><a href='/dept'>按系</a>",
    );
    site.page("/?page=2", "<title>教师名单</title><p>李四</p>");
    site.page("/dept", "<title>按系</title><p>王五</p>");
    let provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    route_all(provider, "list");
    let crawl = queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    for n in 0..queue::LIST_LIMIT {
        sqlx::query(
            r#"INSERT INTO crawl_page (crawl_id, url, scope_url, kind, mode, status, text)
            VALUES (?, ?, ?, 'list', 'http', 'success', '')"#,
        )
        .bind(crawl)
        .bind(site.url(&format!("/read/{n}")).as_str())
        .bind(site.url("/").as_str())
        .execute(&db)
        .await
        .unwrap();
    }
    let pages = Pages {
        writes: db::Writes::default(),
        http: Fetcher::local(site.port),
        browser: None,
        recognize: super::recognize,
    };
    drain(&db, &pages).await;
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM candidate ORDER BY id")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(names, ["张三", "李四"]);
    let limits: String = sqlx::query_scalar("SELECT limits FROM crawl")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(limits, r#"["找名单已有 60 页没找到新导师，后面的没有读"]"#);
}
