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
    let net = net::Net::new(vec![]).unwrap();
    let turns = Turns::default();
    while let Some(claim) = queue::next(db).await.unwrap() {
        let outcome = process(
            db,
            &net,
            pages,
            &turns,
            &claim,
            &Cooldown::new(Arc::default()),
        )
        .await
        .unwrap();
        assert!(matches!(outcome, Outcome::Done(_)));
        assert!(save(db, &claim, &outcome).await.unwrap());
    }
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
        let row: (String, String, String) =
            sqlx::query_as("SELECT status,mode,text FROM crawl_page WHERE id=?")
                .bind(claim.id)
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(
            row,
            ("failed".into(), "webview".into(), "HTTP 已读文字".into())
        );
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
        assert_eq!(after.2, "HTTP 已读文字");
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
