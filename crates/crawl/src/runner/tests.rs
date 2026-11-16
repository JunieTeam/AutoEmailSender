use super::*;
use crate::{
    extract,
    review::{CandidateFilter, CandidateQuery, CandidateSelection, ReviewInput},
};
use mentor::{MentorFields, import::Decisions};
use provider::{Provider, Response};
use serde_json::{Value, json};
use std::sync::Mutex;
use test_fixtures::provider;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    sync::mpsc,
};
// Signals and database state determine progress; this only bounds hangs under load.
const GUARD_TIMEOUT: Duration = Duration::from_secs(60);
type Files = Arc<Mutex<HashMap<String, (u16, &'static str, Vec<u8>)>>>;
struct Site {
    port: u16,
    pages: Arc<Mutex<HashMap<String, (u16, String)>>>,
    /// By path, with their status and content type; status 0 is never answered.
    files: Files,
    visits: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Site {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Site {
    async fn new() -> Self {
        Self::start(false).await
    }
    async fn start(tls: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let pages = Arc::new(Mutex::new(HashMap::<String, (u16, String)>::new()));
        let files = Files::default();
        let visits = Arc::new(Mutex::new(Vec::new()));
        let acceptor = tls.then(|| {
            let cert =
                rcgen::generate_simple_self_signed(vec!["school.example.edu".into()]).unwrap();
            let key =
                net::rustls::pki_types::PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der())
                    .into();
            let config = net::rustls::ServerConfig::builder_with_provider(Arc::new(
                net::rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert.cert.der().clone()], key)
            .unwrap();
            tokio_rustls::TlsAcceptor::from(Arc::new(config))
        });
        let (p, f, v) = (pages.clone(), files.clone(), visits.clone());
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let (p, f, v) = (p.clone(), f.clone(), v.clone());
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let mut first = [0];
                    if stream.peek(&mut first).await.unwrap() == 0 {
                        return;
                    }
                    // Without TLS the handshake is dropped, as by a website only on HTTP.
                    if first[0] == 22 {
                        // The trusted client rejects the fixture certificate before retrying.
                        if let Some(acceptor) = acceptor
                            && let Ok(stream) = acceptor.accept(stream).await
                        {
                            Self::serve(stream, p, f, v, true).await;
                        }
                        return;
                    }
                    Self::serve(stream, p, f, v, false).await;
                });
            }
        });
        Self {
            port,
            pages,
            files,
            visits,
            task,
        }
    }
    async fn serve<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
        stream: S,
        pages: Arc<Mutex<HashMap<String, (u16, String)>>>,
        files: Files,
        visits: Arc<Mutex<Vec<String>>>,
        tls: bool,
    ) {
        let mut io = BufReader::new(stream);
        let mut line = String::new();
        io.read_line(&mut line).await.unwrap();
        let path = line.split_whitespace().nth(1).unwrap().to_owned();
        let mut host = String::new();
        loop {
            let mut h = String::new();
            io.read_line(&mut h).await.unwrap();
            if h == "\r\n" {
                break;
            }
            if let Some((key, value)) = h.split_once(':')
                && key.eq_ignore_ascii_case("host")
            {
                host = value.trim().into();
            }
        }
        let scheme = if tls { "https" } else { "http" };
        let url = format!("{scheme}://{host}{path}");
        visits.lock().unwrap().push(if tls {
            format!("https:{path}")
        } else {
            path.clone()
        });
        let file = files.lock().unwrap().get(&path).cloned();
        if let Some((status, kind, body)) = file {
            if status == 0 {
                std::future::pending::<()>().await;
            }
            let head = format!(
                "HTTP/1.1 {status} Reply\r\nConnection: close\r\nContent-Type: {kind}\r\n\
                Content-Length: {}\r\n\r\n",
                body.len()
            );
            let _ = io.write_all(&[head.as_bytes(), &body].concat()).await;
            return;
        }
        let (status, body) = {
            let pages = pages.lock().unwrap();
            pages
                .get(&url)
                .or_else(|| pages.get(&path))
                .cloned()
                .unwrap_or((404, "not found".into()))
        };
        let response = format!(
            "HTTP/1.1 {status} Reply\r\nConnection: close\r\n{}Content-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\r\n{body}",
            if status == 302 {
                format!("Location: {body}\r\n")
            } else {
                String::new()
            },
            body.len()
        );
        let _ = io.write_all(response.as_bytes()).await;
    }
    fn page(&self, path: &str, body: &str) {
        self.pages
            .lock()
            .unwrap()
            .insert(path.into(), (200, body.into()));
    }
    fn url(&self, path: &str) -> PageUrl {
        PageUrl::parse(&format!("http://school.example.edu:{}{path}", self.port)).unwrap()
    }
    fn file(&self, path: &str, kind: &'static str, body: Vec<u8>) {
        self.files
            .lock()
            .unwrap()
            .insert(path.into(), (200, kind, body));
    }
    fn redirect(&self, path: &str, to: &str) {
        self.pages
            .lock()
            .unwrap()
            .insert(path.into(), (302, to.into()));
    }
}
async fn config(db: &SqlitePool, address: &str) -> i64 {
    let f = model::ModelForm {
        name: "假模型".into(),
        provider: "custom".into(),
        base_url: address.into(),
        model: "fixture".into(),
        region: None,
        currency: None,
        prices: model::pricing::Prices::default(),
        off_peak: model::pricing::Prices::default(),
    }
    .check()
    .unwrap();
    let id = model::create(db, &f, "fixture", 1).await.unwrap();
    model::agree(db, id, &f.source).await.unwrap();
    id
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn extraction_requests_a_json_object(db: SqlitePool) {
    let mut provider = Provider::start().await.unwrap();
    let id = config(&db, &provider.address).await;
    let url = PageUrl::parse("https://school.example.edu/").unwrap();
    queue::start(&db, &[url], id, 1).await.unwrap();
    let claim = queue::next(&db).await.unwrap().unwrap();
    let mut chosen = model::selected(&db, Some(id), model::Feature::Crawl)
        .await
        .unwrap()
        .unwrap();
    chosen.access.provider = model::providers::find("deepseek");
    let reader = Reader {
        writes: &db::Writes::default(),
        db: &db,
        net: &net::Net::new(vec![]).unwrap(),
        turns: &Turns::default(),
        claim: &claim,
        chosen: &chosen,
        cooldown: &Cooldown::new(Arc::default()),
        failed: watch::Sender::new(false),
    };
    let ask = reader.ask::<extract::Route>("route", extract::ROUTE, json!({}));
    let respond = async {
        let request = provider.requests.recv().await.unwrap();
        assert_eq!(
            request.body["response_format"],
            json!({"type":"json_object"})
        );
        request
            .respond
            .send(Response::answer(r#"{"kind":"other","next":[]}"#))
            .unwrap();
    };
    let (result, ()) = tokio::join!(ask, respond);
    assert!(matches!(result.unwrap().kind, extract::Kind::Other));
}

/// The links a route request offers, written `[text](number)`, as text and number.
fn offered(data: &Value) -> Vec<(String, usize)> {
    let link = |l: &Value| {
        let (text, n) = l.as_str()?.strip_prefix('[')?.rsplit_once("](")?;
        Some((text.to_owned(), n.strip_suffix(')')?.parse().ok()?))
    };
    data["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| link(l).unwrap())
        .collect()
}
fn listed(name: &str, profile: Option<usize>, emails: Vec<&str>) -> Value {
    json!({"n": name, "t": "教授", "p": profile, "e": emails})
}
fn person(name: &str, email: &str) -> Value {
    json!({
        "name": name,
        "aliases": [],
        "emails": [email, "invented@example.edu"],
        "affiliations": [],
        "homepages": [],
        "research": "机器学习",
        "papers": ["论文一"]
    })
}
/// What the model says of a page read for a known person: it is theirs.
fn targeted(email: &str) -> Value {
    json!({
        "relation": "matched",
        "aliases": [],
        "emails": [email, "invented@example.edu"],
        "affiliations": [],
        "homepages": [],
        "research": "机器学习",
        "papers": ["论文一"],
        "names": [],
        "next": []
    })
}
async fn changed_until(
    db: &SqlitePool,
    id: i64,
    events: &mut mpsc::UnboundedReceiver<()>,
    state: &str,
) {
    tokio::time::timeout(GUARD_TIMEOUT, async {
        loop {
            let actual = sqlx::query_scalar::<_, String>("SELECT state FROM crawl WHERE id=?")
                .bind(id)
                .fetch_one(db)
                .await
                .unwrap();
            if actual == state {
                break;
            }
            events.recv().await.unwrap();
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn home_categories_pagination_profiles_evidence_and_retry_end_to_end() {
    let (dir, db) = test_fixtures::db::open().await.unwrap();
    let site = Site::new().await;
    site.page(
        "/",
        "<title>学院</title><a href='/list'>师资队伍</a><a href='/news'>学院新闻</a>",
    );
    site.page(
        "/list",
        &format!(
            "<title>教师名单</title><article><p><a href='/alice'>张三</a> a@school.example.edu</p>\
            <p><a href='http://outside.example.org:{}/bob'>李四</a> b@school.example.edu</p>\
            </article><a href='/category'>副教授</a><nav><a href='/list?page=1'>1</a><a \
            href='/list?page=2'>2</a></nav><a href='/missing'>讲师名单</a><a href='/browser'>\
            动态名单</a>",
            site.port
        ),
    );
    site.page(
        "/category",
        "<title>副教授</title><p><a href='/alternate-alice'>张三</a> a@school.example.edu</p>",
    );
    site.page(
        "/list?page=1",
        "<title>教师名单一</title><p><a href='/alice'>张三</a></p>",
    );
    site.page(
        "/list?page=2",
        "<title>教师名单二</title><p>王五 c@school.example.edu</p>",
    );
    site.page(
        "/alice",
        "<title>张三</title><p>张三 a@school.example.edu 机器学习</p>",
    );
    site.page(
        "/bob",
        "<title>李四</title><p>李四 b@school.example.edu 机器学习</p><a href='/must-not-visit'>\
        下一页</a>",
    );
    site.page(
        "/browser",
        "<div id='app'></div><script type='module' src='app.js'></script>",
    );
    let mut provider = Provider::start().await.unwrap();
    let id = config(&db, &provider.address).await;
    let model = tokio::spawn(async move {
        while let Some(r) = provider.requests.recv().await {
            let system = r.body["messages"][0]["content"].as_str().unwrap();
            let data: Value =
                serde_json::from_str(r.body["messages"][1]["content"].as_str().unwrap()).unwrap();
            let title = data["title"].as_str().unwrap_or("");
            let answer = if system.starts_with("你整理学院") {
                let next: Vec<_> = offered(&data)
                    .into_iter()
                    .filter(|(text, _)| {
                        ["师资队伍", "副教授", "讲师名单", "动态名单"].contains(&text.as_str())
                    })
                    .map(|(_, id)| id)
                    .chain(std::iter::once(99999))
                    .collect();
                json!({"kind":if title=="学院"{"entry"}else{"list"},"next":next})
            } else if system.starts_with("你从教师") {
                if title == "教师名单二" {
                    json!({"people":[listed("王五",None,vec!["c@school.example.edu"])]})
                } else if title == "教师名单" {
                    json!({
                        "people": [
                            listed(
                                "张三",
                                Some(1),
                                vec![
                                    "a@school.example.edu",
                                    "b@school.example.edu",
                                    "invented@example.edu"
                                ]
                            ),
                            listed("李四", Some(2), vec!["a@school.example.edu"])
                        ]
                    })
                } else {
                    json!({"people":[listed("张三",Some(1),vec!["a@school.example.edu"])]})
                }
            } else {
                targeted(if title == "张三" {
                    "a@school.example.edu"
                } else {
                    "b@school.example.edu"
                })
            };
            r.respond.send(Response::answer(&answer.to_string())).ok();
        }
    });
    let crawl = queue::start(&db, &[site.url("/")], id, 1).await.unwrap();
    let runner = Crawler::default();
    let (tx, mut events) = mpsc::unbounded_channel();
    let (done, mut finished) = mpsc::unbounded_channel();
    let task = tokio::spawn(run_with_pages(
        db.clone(),
        net::Net::new(vec![]).unwrap(),
        Arc::new(Pages::local(site.port)),
        runner.clone(),
        Turns::default(),
        move || {
            tx.send(()).ok();
        },
        move |r| {
            done.send(r).ok();
        },
    ));
    changed_until(&db, crawl, &mut events, "done").await;
    let result = finished.recv().await.unwrap();
    assert!(result.message.contains("3 位候选"));
    assert!(result.message.contains("2 个页面未打开"));
    let candidates = review::list(
        &db,
        &CandidateQuery {
            selection: CandidateSelection {
                crawl_id: Some(crawl),
                processed: false,
                filter: CandidateFilter::All,
                keyword: None,
            },
            page: 1,
        },
    )
    .await
    .unwrap();
    assert_eq!(candidates.total, 3);
    for c in candidates.candidates {
        assert!(!c.emails.contains(&"invented@example.edu".into()));
        if c.name == "张三" {
            let detail = review::detail(&db, c.id).await.unwrap().unwrap();
            let list = detail
                .evidence
                .iter()
                .find(|e| e.url.ends_with("/list"))
                .unwrap();
            assert_eq!(list.emails, vec!["a@school.example.edu"]);
            assert_eq!(c.emails, vec!["a@school.example.edu"]);
        }
    }
    assert!(
        !site
            .visits
            .lock()
            .unwrap()
            .iter()
            .any(|p| p == "/news" || p == "/must-not-visit" || p == "/alternate-alice")
    );
    let detail = queue::detail(&db, crawl, 1, false).await.unwrap().unwrap();
    assert!(
        detail
            .pages
            .iter()
            .any(|p| p.status == PageStatus::NeedsBrowser)
    );
    let s = &detail.summary;
    assert_eq!(
        (s.profiles, s.profiles_done, s.with_email, s.pending),
        (2, 2, 3, 3)
    );
    let failed = queue::detail(&db, crawl, 1, true).await.unwrap().unwrap();
    assert_eq!(failed.page_total, 2);
    assert!(failed.pages.iter().all(|p| p.status != PageStatus::Success));
    site.page(
        "/missing",
        "<title>教师名单二</title><p>王五 c@school.example.edu</p>",
    );
    let failed = detail
        .pages
        .iter()
        .find(|p| p.url.ends_with("/missing"))
        .unwrap()
        .id;
    assert_eq!(queue::retry(&db, crawl, Some(failed), 2).await.unwrap(), 1);
    runner.wake();
    changed_until(&db, crawl, &mut events, "done").await;
    let detail = queue::detail(&db, crawl, 1, false).await.unwrap().unwrap();
    assert_eq!(
        detail.pages.iter().find(|p| p.id == failed).unwrap().status,
        PageStatus::Success
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM candidate WHERE crawl_id=?")
            .bind(crawl)
            .fetch_one(&db)
            .await
            .unwrap(),
        3
    );
    assert!(
        sqlx::query_scalar::<_, i64>(
            "SELECT sum(input_tokens+output_tokens) FROM usage WHERE crawl_id=?"
        )
        .bind(crawl)
        .fetch_one(&db)
        .await
        .unwrap()
            > 0
    );
    task.abort();
    model.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(model.await.unwrap_err().is_cancelled());
    db.close().await;
    dir.close().unwrap();
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn claims_pause_resume_cancel_restart_and_limits(db: SqlitePool) {
    let mut provider = Provider::start().await.unwrap();
    let config = config(&db, &provider.address).await;
    let site = Site::new().await;
    site.page("/", "<title>张三</title><p>张三 a@school.example.edu</p>");
    let id = queue::start(&db, &[site.url("/")], config, 1)
        .await
        .unwrap();
    let old = queue::next(&db).await.unwrap().unwrap();
    queue::control(&db, id, "pause", 2).await.unwrap();
    assert!(
        !save(&db, &old, &Outcome::Failed(Failure::new("旧错误")))
            .await
            .unwrap()
    );
    queue::control(&db, id, "resume", 3).await.unwrap();
    let interrupted = queue::next(&db).await.unwrap().unwrap();
    queue::recover(&db).await.unwrap();
    assert!(
        !save(&db, &interrupted, &Outcome::Failed(Failure::new("旧错误")))
            .await
            .unwrap()
    );
    let next = queue::next(&db).await.unwrap().unwrap();
    queue::control(&db, id, "cancel", 4).await.unwrap();
    assert!(
        !save(&db, &next, &Outcome::Failed(Failure::new("旧错误")))
            .await
            .unwrap()
    );
    let id = queue::start(&db, &[site.url("/")], config, 5)
        .await
        .unwrap();
    let c = queue::next(&db).await.unwrap().unwrap();
    let process = tokio::spawn({
        let db = db.clone();
        let port = site.port;
        let c = c.clone();
        async move {
            super::process(
                &db,
                &net::Net::new(vec![]).unwrap(),
                &Pages::local(port),
                &Turns::default(),
                &c,
                &Cooldown::new(Arc::default()),
            )
            .await
            .unwrap()
        }
    });
    let r = provider.requests.recv().await.unwrap();
    r.respond
        .send(Response::Refused {
            status: 401,
            message: "invalid key".into(),
            retry_after: None,
        })
        .ok();
    let result = process.await.unwrap();
    save(&db, &c, &result).await.unwrap();
    assert_eq!(
        queue::list(&db, 1).await.unwrap().crawls[0].state,
        queue::CrawlState::Paused
    );
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await.unwrap();
    for n in 0..70 {
        queue::enqueue(
            &mut tx,
            id,
            &site.url(&format!("/category/{n}")),
            &site.url("/"),
            1,
            PageKind::Entry,
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM crawl_page WHERE crawl_id=?")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap(),
        60
    );
    assert!(
        !queue::detail(&db, id, 1, false)
            .await
            .unwrap()
            .unwrap()
            .limits
            .is_empty()
    );
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn candidate_identity_person_limit_and_review_transactions(db: SqlitePool) {
    let site = Site::new().await;
    let config = config(&db, "http://127.0.0.1:1").await;
    let crawl = queue::start(&db, &[site.url("/")], config, 1)
        .await
        .unwrap();
    let claim = queue::next(&db).await.unwrap().unwrap();
    let existing = mentor::create(
        &db,
        &MentorFields {
            name: "李四".into(),
            emails: vec!["li@example.edu".into()],
            research: Some("本地方向".into()),
            ..Default::default()
        },
        1,
    )
    .await
    .unwrap();
    let other = mentor::create(
        &db,
        &MentorFields {
            name: "王五".into(),
            emails: vec!["wang@example.edu".into()],
            ..Default::default()
        },
        1,
    )
    .await
    .unwrap();
    let third = mentor::create(
        &db,
        &MentorFields {
            name: "赵六".into(),
            emails: vec!["zhao@example.edu".into()],
            ..Default::default()
        },
        1,
    )
    .await
    .unwrap();
    let same = mentor::create(
        &db,
        &MentorFields {
            name: "张三".into(),
            ..Default::default()
        },
        1,
    )
    .await
    .unwrap();
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await.unwrap();
    for (name, emails, profile) in [
        ("新导师", vec!["new@example.edu"], Some("/new")),
        (
            "李四",
            vec!["li@example.edu", "extra@school.example.edu"],
            Some("/li"),
        ),
        ("张三", vec![], None),
        ("冲突", vec!["zhao@example.edu", "wang@example.edu"], None),
    ] {
        let url = profile.map_or_else(|| site.url(&format!("/people/{name}")), |p| site.url(p));
        review::save(
            &mut tx,
            crawl,
            review::Source::Profile {
                id: claim.id,
                own: true,
                target: None,
            },
            url.as_str(),
            Person {
                fields: MentorFields {
                    name: name.into(),
                    emails: emails.into_iter().map(Into::into).collect(),
                    research: Some("整理方向".into()),
                    ..Default::default()
                },
                papers: vec!["论文".into()],
                profile: profile.map(|p| site.url(p)),
                recognized: vec![],
            },
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    let q = CandidateQuery {
        selection: CandidateSelection {
            crawl_id: Some(crawl),
            processed: false,
            filter: CandidateFilter::All,
            keyword: None,
        },
        page: 1,
    };
    let page = review::list(&db, &q).await.unwrap();
    assert_eq!(page.total, 4);
    let new = page.candidates.iter().find(|c| c.name == "新导师").unwrap();
    assert_eq!(new.category, CandidateFilter::New);
    let supplement = page.candidates.iter().find(|c| c.name == "李四").unwrap();
    assert_eq!(supplement.category, CandidateFilter::Supplement);
    let maybe = page.candidates.iter().find(|c| c.name == "张三").unwrap();
    assert_eq!(maybe.category, CandidateFilter::Maybe);
    let conflict = page.candidates.iter().find(|c| c.name == "冲突").unwrap();
    let reason = conflict.issue.as_ref().unwrap();
    for (name, id) in [("王五", other), ("赵六", third)] {
        assert!(reason.contains(name));
        assert!(reason.contains(&format!("编号 {id}")));
    }
    let input = ReviewInput {
        ids: vec![new.id, supplement.id, maybe.id, conflict.id],
        decisions: Decisions::default(),
    };
    let preview = review::preview(&db, &input).await.unwrap();
    assert_eq!(preview.maybe_same[0].selected, Some(same));
    let result = review::accept(&db, &input, 2).await.unwrap();
    assert_eq!(result.accepted, 3);
    assert_eq!(result.skipped, 1);
    let updated = mentor::get(&db, existing).await.unwrap().unwrap();
    assert_eq!(updated.research.as_deref(), Some("本地方向"));
    assert!(updated.emails.contains(&"extra@school.example.edu".into()));
    assert_eq!(review::accept(&db, &input, 3).await.unwrap().accepted, 0);
    assert_eq!(
        review::detail(&db, maybe.id)
            .await
            .unwrap()
            .unwrap()
            .candidate
            .mentor_id,
        Some(same)
    );
    assert_eq!(review::reject(&db, &[conflict.id]).await.unwrap(), 1);
    assert!(mentor::get(&db, other).await.unwrap().is_some());
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await.unwrap();
    for n in 0..1001 {
        review::save(
            &mut tx,
            crawl,
            review::Source::Profile {
                id: claim.id,
                own: true,
                target: None,
            },
            site.url(&format!("/limit/{n}")).as_str(),
            Person {
                fields: MentorFields {
                    name: format!("候选{n}"),
                    ..Default::default()
                },
                papers: vec![],
                profile: Some(site.url(&format!("/limit/{n}"))),
                recognized: vec![],
            },
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM candidate WHERE crawl_id=?")
            .bind(crawl)
            .fetch_one(&db)
            .await
            .unwrap(),
        1000
    );
    queue::delete(&db, crawl).await.unwrap();
    assert!(review::detail(&db, new.id).await.unwrap().is_some());
    assert_eq!(
        review::detail(&db, new.id)
            .await
            .unwrap()
            .unwrap()
            .candidate
            .crawl_id,
        None
    );
}
#[test]
fn segments_preserve_blocks_and_external_output_is_checked() {
    assert_eq!(
        queue::starts(
            "HTTPS://SCHOOL.EXAMPLE.EDU:443/list#one\nhttps://school.example.edu/list#two"
        )
        .unwrap()
        .len(),
        1
    );
    assert!(queue::starts("https://school.example.edu/list\nhttp://127.0.0.1/").is_err());
    let url = PageUrl::parse("https://school.example.edu/list").unwrap();
    let m = crate::parse(
        &url,
        &format!(
            "<p>{}</p><p>{}</p><p>李四 a@example.edu</p>",
            "甲".repeat(4000),
            "乙".repeat(4000)
        ),
    );
    assert_eq!(extract::segments(&m), [0..1, 1..2, 2..3]);
    assert!(
        extract::parse::<extract::Route>(
            r#"{"kind":"list","next":["https://evil.example"]}"#,
            true
        )
        .is_err()
    );
    let people = extract::listed(
        &m,
        vec![extract::Listed {
            name: "李四".into(),
            title: None,
            profile: Some(999),
            emails: vec!["invented@example.edu".into()],
            research: None,
        }],
        &Scope::new(&url),
        None,
    )
    .unwrap();
    assert_eq!(people.len(), 1);
    assert!(people[0].fields.emails.is_empty());
    assert!(people[0].profile.is_none());
}

#[tokio::test]
async fn crawling_matching_and_drafting_share_three_actual_calls() {
    let (dir, db) = test_fixtures::db::open().await.unwrap();
    let mut p = Provider::start().await.unwrap();
    let config = config(&db, &p.address).await;
    let site = Site::new().await;
    site.page(
        "/crawl",
        "<title>张三</title><p>张三 a@school.example.edu 机器学习</p>",
    );
    site.page(
        "/enrich",
        "<title>张三</title><p>张三 a@school.example.edu 机器学习</p>",
    );
    queue::start(&db, &[site.url("/crawl")], config, 1)
        .await
        .unwrap();
    let (_, candidate) = enrichment::candidate(&db, &site, "/enrich").await;
    crate::enrich::start(&db, &[candidate], 1).await.unwrap();
    let mentor = mentor::create(
        &db,
        &MentorFields {
            name: "导师".into(),
            emails: vec!["mentor@example.edu".into()],
            research: Some("机器学习".into()),
            ..Default::default()
        },
        1,
    )
    .await
    .unwrap();
    let ids = vec![mentor];
    sqlx::query("UPDATE profile SET research_interests='机器学习'")
        .execute(&db)
        .await
        .unwrap();
    mentor::matching::enqueue(&db, &ids, false, true, 1)
        .await
        .unwrap();
    task::create(
        &db,
        &task::NewTask {
            name: "写信".into(),
            kind: task::Kind::First,
            ids,
            writer: task::Writer::Ai,
            template_id: None,
            template_source_id: None,
            source_task_id: None,
            model_config_id: Some(config),
            references: vec![],
            attachments: vec![],
            schedule: email::schedule::Schedule::Now,
        },
        1,
    )
    .await
    .unwrap();
    let net = net::Net::new(vec![]).unwrap();
    let turns = Turns::default();
    let crawl = tokio::spawn(run_with_pages(
        db.clone(),
        net.clone(),
        Arc::new(Pages::local(site.port)),
        Crawler::default(),
        turns.clone(),
        || {},
        |_| {},
    ));
    let matching = tokio::spawn(matching::run(
        db.clone(),
        db::Writes::default(),
        net.clone(),
        matching::Matcher::default(),
        turns.clone(),
        || {},
        |_| {},
    ));
    let writing = tokio::spawn(task::generation::run(
        db.clone(),
        db::Writes::default(),
        net,
        task::generation::Generator::default(),
        turns,
        || {},
        |_| {},
    ));
    let mut active = Vec::new();
    for _ in 0..3 {
        active.push(p.requests.recv().await.unwrap());
    }
    tokio::time::pause();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), p.requests.recv())
            .await
            .is_err()
    );
    tokio::time::resume();
    let purpose = |request: &provider::Request| {
        let system = request.body["messages"][0]["content"].as_str().unwrap();
        if system.starts_with("你整理学院") {
            "crawl"
        } else if system.starts_with("你整理 target") {
            "enrich"
        } else if system.contains("分数不是录取") {
            "matching"
        } else if system.starts_with("你帮一位学生") {
            "drafting"
        } else {
            panic!("unexpected model request");
        }
    };
    let mut seen = active
        .iter()
        .map(purpose)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(seen.len(), 3);
    // These entries finish in one call, so releasing one cannot start a follow-up request.
    let index = active
        .iter()
        .position(|r| matches!(purpose(r), "matching" | "enrich"))
        .unwrap();
    let released = active.remove(index);
    let answer = if purpose(&released) == "matching" {
        json!({"score":80,"reason":"方向相关","directions":[],"highlights":[],"risks":[]})
    } else {
        targeted("a@school.example.edu")
    };
    released
        .respond
        .send(Response::answer(&answer.to_string()))
        .ok()
        .unwrap();
    let waiting = tokio::time::timeout(GUARD_TIMEOUT, p.requests.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(seen.insert(purpose(&waiting)));
    assert_eq!(
        seen,
        std::collections::BTreeSet::from(["crawl", "enrich", "matching", "drafting"])
    );
    for worker in [&crawl, &matching, &writing] {
        worker.abort();
    }
    for worker in [crawl, matching, writing] {
        assert!(worker.await.unwrap_err().is_cancelled());
    }
    db.close().await;
    dir.close().unwrap();
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn going_on_after_a_lasting_limit_paused_the_crawl_asks_again_at_once(db: SqlitePool) {
    let site = Site::new().await;
    site.page("/", include_str!("../../tests/pages/toronto.html"));
    let mut p = Provider::start().await.unwrap();
    let config = config(&db, &p.address).await;
    let id = queue::start(&db, &[site.url("/")], config, 1)
        .await
        .unwrap();
    let handle = Crawler::default();
    let (tx, mut events) = mpsc::unbounded_channel();
    let task = tokio::spawn(run_with_pages(
        db.clone(),
        net::Net::new(vec![]).unwrap(),
        Arc::new(Pages::local(site.port)),
        handle.clone(),
        Turns::default(),
        move || {
            tx.send(()).ok();
        },
        |_| {},
    ));
    p.requests
        .recv()
        .await
        .unwrap()
        .respond
        .send(Response::Refused {
            status: 429,
            message: "coding plan expired".into(),
            retry_after: Some(601),
        })
        .ok();
    changed_until(&db, id, &mut events, "paused").await;
    queue::control(&db, id, "resume", 3).await.unwrap();
    handle.restart(id);
    tokio::time::timeout(GUARD_TIMEOUT, p.requests.recv())
        .await
        .unwrap()
        .unwrap();
    task.abort();
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn late_results_restart_and_retry_after_preserve_work(db: SqlitePool) {
    let site = Site::new().await;
    site.page("/", include_str!("../../tests/pages/toronto.html"));
    let mut p = Provider::start().await.unwrap();
    let config = config(&db, &p.address).await;
    let id = queue::start(&db, &[site.url("/")], config, 1)
        .await
        .unwrap();
    let handle = Crawler::default();
    let (tx, mut events) = mpsc::unbounded_channel();
    let start = |tx: mpsc::UnboundedSender<()>| {
        tokio::spawn(run_with_pages(
            db.clone(),
            net::Net::new(vec![]).unwrap(),
            Arc::new(Pages::local(site.port)),
            handle.clone(),
            Turns::default(),
            move || {
                tx.send(()).ok();
            },
            |_| {},
        ))
    };
    let task = start(tx.clone());
    let old = p.requests.recv().await.unwrap();
    queue::control(&db, id, "pause", 2).await.unwrap();
    handle.wake();
    old.respond
        .send(Response::answer(r#"{"kind":"profile","next":[]}"#))
        .ok();
    queue::control(&db, id, "resume", 3).await.unwrap();
    handle.restart(id);
    let interrupted = p.requests.recv().await.unwrap();
    task.abort();
    let task = start(tx);
    let current = p.requests.recv().await.unwrap();
    interrupted
        .respond
        .send(Response::answer(r#"{"kind":"profile","next":[]}"#))
        .ok();
    let guard = tokio::spawn(async {
        loop {
            tokio::task::yield_now().await;
        }
    });
    tokio::time::pause();
    while events.try_recv().is_ok() {}
    current
        .respond
        .send(Response::Refused {
            status: 429,
            message: "slow down".into(),
            retry_after: Some(60),
        })
        .ok();
    loop {
        events.recv().await.unwrap();
        let queued = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM crawl_page WHERE crawl_id=? AND work_state='queued'",
        )
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap();
        if queued == 1 {
            break;
        }
    }
    tokio::time::advance(Duration::from_secs(59)).await;
    assert!(p.requests.try_recv().is_err());
    tokio::time::advance(Duration::from_secs(2)).await;
    tokio::time::resume();
    guard.abort();
    let current = tokio::time::timeout(GUARD_TIMEOUT, p.requests.recv())
        .await
        .unwrap()
        .unwrap();
    current
        .respond
        .send(Response::answer(r#"{"kind":"profile","next":[]}"#))
        .ok();
    let profile = p.requests.recv().await.unwrap();
    profile
        .respond
        .send(Response::answer(
            &person("Geoffrey Hinton", "geoffrey.hinton@gmail.com").to_string(),
        ))
        .ok();
    changed_until(&db, id, &mut events, "done").await;
    let candidates = review::list(
        &db,
        &CandidateQuery {
            selection: CandidateSelection {
                crawl_id: Some(id),
                processed: false,
                filter: CandidateFilter::All,
                keyword: None,
            },
            page: 1,
        },
    )
    .await
    .unwrap();
    assert_eq!(candidates.total, 1);
    assert_eq!(
        candidates.candidates[0].emails,
        vec!["geoffrey.hinton@gmail.com"]
    );
    let detail = queue::detail(&db, id, 1, false).await.unwrap().unwrap();
    assert!(detail.runs.len() >= 3);
    assert!(
        sqlx::query_scalar::<_, String>("SELECT text FROM crawl_page WHERE id=?")
            .bind(detail.pages[0].id)
            .fetch_one(&db)
            .await
            .unwrap()
            .contains("Hinton")
    );
    task.abort();
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn review_explicit_targets_differences_and_trash_follow_the_shared_rules(db: SqlitePool) {
    let site = Site::new().await;
    let config = config(&db, "http://127.0.0.1:1").await;
    let crawl = queue::start(&db, &[site.url("/")], config, 1)
        .await
        .unwrap();
    let page = queue::next(&db).await.unwrap().unwrap();
    let mut locals = Vec::new();
    for research in ["本地方向甲", "本地方向乙"] {
        locals.push(
            mentor::create(
                &db,
                &MentorFields {
                    name: "同名导师".into(),
                    research: Some(research.into()),
                    ..Default::default()
                },
                1,
            )
            .await
            .unwrap(),
        );
    }
    let trash = mentor::create(
        &db,
        &MentorFields {
            name: "回收站导师".into(),
            emails: vec!["trash@example.edu".into()],
            ..Default::default()
        },
        1,
    )
    .await
    .unwrap();
    mentor::trash::trash(&db, &[trash], 2).await.unwrap();
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await.unwrap();
    for (name, emails) in [
        ("同名导师", vec![]),
        ("回收站导师", vec!["trash@example.edu".into()]),
    ] {
        review::save(
            &mut tx,
            crawl,
            review::Source::Profile {
                id: page.id,
                own: true,
                target: None,
            },
            site.url(&format!("/people/{name}")).as_str(),
            Person {
                fields: MentorFields {
                    name: name.into(),
                    emails,
                    research: Some("新的研究方向".into()),
                    ..Default::default()
                },
                papers: vec![],
                profile: None,
                recognized: vec![],
            },
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    let ids = sqlx::query_scalar::<_, i64>("SELECT id FROM candidate ORDER BY id")
        .fetch_all(&db)
        .await
        .unwrap();
    let mut input = ReviewInput {
        ids: ids.clone(),
        decisions: Decisions::default(),
    };
    let p = review::preview(&db, &input).await.unwrap();
    assert_eq!(p.maybe_same[0].candidates.len(), 2);
    assert_eq!(p.maybe_same[0].selected, None);
    assert_eq!(p.trashed.len(), 1);
    input.decisions.matches.push(mentor::import::GroupChoice {
        row: 1,
        id: Some(locals[1]),
    });
    input.decisions.take = review::preview(&db, &input).await.unwrap().differences;
    let result = review::accept(&db, &input, 3).await.unwrap();
    assert_eq!((result.accepted, result.skipped), (1, 1));
    assert_eq!(
        mentor::get(&db, locals[1])
            .await
            .unwrap()
            .unwrap()
            .research
            .as_deref(),
        Some("新的研究方向")
    );
    assert_eq!(
        mentor::get(&db, locals[0])
            .await
            .unwrap()
            .unwrap()
            .research
            .as_deref(),
        Some("本地方向甲")
    );
    input.decisions.restore.push(trash);
    assert_eq!(review::accept(&db, &input, 4).await.unwrap().accepted, 1);
    assert!(!mentor::get(&db, trash).await.unwrap().unwrap().trashed);
    // Removing an earlier selected candidate cannot redirect a saved group choice to another row.
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await.unwrap();
    for n in 0..2 {
        review::save(
            &mut tx,
            crawl,
            review::Source::Profile {
                id: page.id,
                own: true,
                target: None,
            },
            site.url(&format!("/same/{n}")).as_str(),
            Person {
                fields: MentorFields {
                    name: "同名导师".into(),
                    research: Some(format!("不同{n}")),
                    ..Default::default()
                },
                papers: vec![],
                profile: Some(site.url(&format!("/same/{n}"))),
                recognized: vec![],
            },
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    let pending =
        sqlx::query_scalar::<_, i64>("SELECT id FROM candidate WHERE review='pending' ORDER BY id")
            .fetch_all(&db)
            .await
            .unwrap();
    let input = ReviewInput {
        ids: pending.clone(),
        decisions: Decisions {
            matches: vec![mentor::import::GroupChoice {
                row: 1,
                id: Some(locals[0]),
            }],
            ..Default::default()
        },
    };
    sqlx::query("DELETE FROM candidate WHERE id=?")
        .bind(pending[0])
        .execute(&db)
        .await
        .unwrap();
    review::accept(&db, &input, 5).await.unwrap();
    assert_ne!(
        review::detail(&db, pending[1])
            .await
            .unwrap()
            .unwrap()
            .candidate
            .mentor_id,
        Some(locals[0])
    );
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_run_with_only_failed_pages_can_retry_without_losing_its_page(db: SqlitePool) {
    let url = PageUrl::parse("https://school.example.edu/").unwrap();
    let model = config(&db, "http://127.0.0.1:1").await;
    let id = queue::start(&db, std::slice::from_ref(&url), model, 1)
        .await
        .unwrap();
    let claim = queue::next(&db).await.unwrap().unwrap();
    save(
        &db,
        &claim,
        &Outcome::Failed(Failure::new("读取网页超时，请重试")),
    )
    .await
    .unwrap();
    assert!(
        finish(&db, now()).await.unwrap()[0]
            .message
            .contains("1 个页面未打开")
    );
    assert_eq!(
        queue::list(&db, 1).await.unwrap().crawls[0].state,
        queue::CrawlState::Failed
    );
    assert_eq!(queue::retry(&db, id, None, 2).await.unwrap(), 1);
    let retried = queue::next(&db).await.unwrap().unwrap();
    assert_eq!(claim.id, retried.id);
    assert_ne!(claim.generation, retried.generation);
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_teacher_s_page_linked_over_http_and_https_is_one_page(db: SqlitePool) {
    // As cse.csu.edu.cn's lists of teachers and of supervisors link the same pages.
    let url = PageUrl::parse("https://cse.csu.edu.cn/szdw/jsml.htm").unwrap();
    let config = config(&db, "http://127.0.0.1:1").await;
    let crawl = queue::start(&db, std::slice::from_ref(&url), config, 1)
        .await
        .unwrap();
    let page = queue::next(&db).await.unwrap().unwrap();
    let profile = |scheme: &str| {
        PageUrl::parse(&format!(
            "{scheme}://faculty.csu.edu.cn/heshiwen/zh_CN/index.htm"
        ))
        .unwrap()
    };
    let person = |name: &str, scheme: &str| Person {
        fields: MentorFields {
            name: name.into(),
            homepages: vec![profile(scheme).as_str().into()],
            ..Default::default()
        },
        papers: vec![],
        profile: Some(profile(scheme)),
        recognized: vec![],
    };
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await.unwrap();
    for scheme in ["http", "https"] {
        let source = review::Source::List { id: page.id };
        (review::save(
            &mut tx,
            crawl,
            source,
            url.as_str(),
            person("何世文", scheme),
        )
        .await)
            .unwrap();
    }
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM candidate")
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    let [he] = ids[..] else {
        panic!("one candidate for both links: {ids:?}")
    };
    // The page read over https proves not to be his, and the next list links it to its teacher.
    review::mismatched(&mut tx, he, &[profile("https").as_str()], 2)
        .await
        .unwrap();
    let source = review::Source::List { id: page.id };
    (review::save(
        &mut tx,
        crawl,
        source,
        url.as_str(),
        person("贺文", "https"),
    )
    .await)
        .unwrap();
    tx.commit().await.unwrap();
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM candidate ORDER BY id")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(names, ["何世文", "贺文"]);
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_page_proved_not_someone_s_is_not_theirs_over_a_scheme_linked_later(db: SqlitePool) {
    let url = PageUrl::parse("https://school.example.edu/szdw").unwrap();
    let config = config(&db, "http://127.0.0.1:1").await;
    let crawl = queue::start(&db, std::slice::from_ref(&url), config, 1)
        .await
        .unwrap();
    let page = queue::next(&db).await.unwrap().unwrap();
    let wrong =
        |scheme: &str| PageUrl::parse(&format!("{scheme}://school.example.edu/wrong")).unwrap();
    let person = |name: &str, scheme: &str, emails: &[&str]| Person {
        fields: MentorFields {
            name: name.into(),
            emails: emails.iter().map(|&e| e.into()).collect(),
            homepages: vec![wrong(scheme).as_str().into()],
            ..Default::default()
        },
        papers: vec![],
        profile: Some(wrong(scheme)),
        recognized: vec![],
    };
    let zhang = ["z@school.example.edu"];
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await.unwrap();
    let source = review::Source::List { id: page.id };
    (review::save(
        &mut tx,
        crawl,
        source,
        url.as_str(),
        person("张三", "https", &zhang),
    )
    .await)
        .unwrap();
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM candidate")
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    // Read over https, the page proves not to be his; another list then links him to it over
    // http, and a third gives it to its teacher over https.
    review::mismatched(&mut tx, ids[0], &[wrong("https").as_str()], 2)
        .await
        .unwrap();
    for p in [person("张三", "http", &zhang), person("李四", "https", &[])] {
        let source = review::Source::List { id: page.id };
        (review::save(&mut tx, crawl, source, url.as_str(), p).await).unwrap();
    }
    tx.commit().await.unwrap();
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM candidate ORDER BY id")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(names, ["张三", "李四"]);
}

/// A site whose `/` lists 张三 at `/info/1/7.htm` and links `/boshi`, which lists him again at
/// `/info/2/8.htm`.
async fn columns() -> Site {
    let site = Site::new().await;
    site.page(
        "/",
        "<title>教授</title><p><a href='/info/1/7.htm'>张三</a></p><a href='/boshi'>博士生导师</a>",
    );
    site.page(
        "/boshi",
        "<title>博士生导师</title><p><a href='/info/2/8.htm'>张三</a></p>",
    );
    site
}

/// Crawls `site` from `/`, its first list giving 张三 `emails`, and 李四 after him where it lists
/// him too, and the second giving 张三 once a candidate has `a@school.example.edu` and the pages
/// read for the first's are done; the model says of each page read for them what `answer` does.
/// Gives how many such pages the model read.
async fn read_for_zhang(
    db: &SqlitePool,
    site: &Site,
    emails: Vec<&'static str>,
    answer: fn(&Value) -> Value,
) -> usize {
    let mut provider = Provider::start().await.unwrap();
    let id = config(db, &provider.address).await;
    let (saved, seen) = watch::channel(());
    let read = Arc::new(Mutex::new(0));
    let model = {
        let (db, read) = (db.clone(), read.clone());
        tokio::spawn(async move {
            while let Some(r) = provider.requests.recv().await {
                let (db, read, mut seen) = (db.clone(), read.clone(), seen.clone());
                let emails = emails.clone();
                tokio::spawn(async move {
                    let system = r.body["messages"][0]["content"].as_str().unwrap();
                    let data: Value =
                        serde_json::from_str(r.body["messages"][1]["content"].as_str().unwrap())
                            .unwrap();
                    let answer = if system.starts_with("你整理学院") {
                        let next: Vec<_> = (offered(&data).into_iter())
                            .filter(|(text, _)| text == "博士生导师")
                            .map(|(_, n)| n)
                            .collect();
                        json!({"kind": "list", "next": next})
                    } else if data["title"] == "教授" {
                        let mut people = vec![listed("张三", Some(1), emails)];
                        if data.to_string().contains("李四") {
                            people.push(listed("李四", Some(2), vec![]));
                        }
                        json!({ "people": people })
                    } else if system.starts_with("你从教师") {
                        while sqlx::query_scalar::<_, i64>(
                            "SELECT EXISTS (SELECT 1 FROM candidate WHERE emails LIKE '%a@school%')
                            AND NOT EXISTS (
                                SELECT 1 FROM crawl_page WHERE kind = 'profile' AND work_state != 'done'
                            )",
                        )
                        .fetch_one(&db)
                        .await
                        .unwrap()
                            == 0
                        {
                            seen.changed().await.unwrap();
                        }
                        json!({"people": [listed("张三", Some(1), vec![])]})
                    } else {
                        *read.lock().unwrap() += 1;
                        answer(&data)
                    };
                    r.respond.send(Response::answer(&answer.to_string())).ok();
                });
            }
        })
    };
    let crawl = queue::start(db, &[site.url("/")], id, 1).await.unwrap();
    let (tx, mut events) = mpsc::unbounded_channel();
    let task = tokio::spawn(run_with_pages(
        db.clone(),
        net::Net::new(vec![]).unwrap(),
        Arc::new(Pages::local(site.port)),
        Crawler::default(),
        Turns::default(),
        move || {
            tx.send(()).ok();
            saved.send_replace(());
        },
        |_| {},
    ));
    changed_until(db, crawl, &mut events, "done").await;
    task.abort();
    model.abort();
    *read.lock().unwrap()
}

#[tokio::test]
async fn a_copy_of_a_teacher_s_page_read_before_is_not_read_again() {
    // As chem.fzu.edu.cn and jidian.cug.edu.cn publish a teacher's page under each column that
    // lists them, at another address each time, naming the column on it, though not right above
    // an address; the former's footer gives the college's address and links its dean's, as each
    // of its teachers' pages does.
    let (dir, db) = test_fixtures::db::open().await.unwrap();
    let site = columns().await;
    site.page(
        "/",
        "<title>教授</title><p><a href='/info/1/7.htm'>张三</a></p>\
        <p><a href='/info/3/9.htm'>李四</a></p><a href='/boshi'>博士生导师</a>",
    );
    let footer = "<div class='footer'><p>邮箱：office@school.example.edu</p>\
        <p><a href='mailto:dean@school.example.edu'>院长信箱</a></p></div>";
    for (path, column) in [("/info/1/7.htm", "教授"), ("/info/2/8.htm", "博士生导师")] {
        site.page(
            path,
            &format!(
                "<title>张三</title><h2>{column}</h2><p>张三</p><p>a@school.example.edu 机器学习</p>\
                {footer}"
            ),
        );
    }
    site.page(
        "/info/3/9.htm",
        &format!("<title>李四</title><p>李四</p><p>li@school.example.edu 数据库</p>{footer}"),
    );
    let read = read_for_zhang(&db, &site, vec![], |data| {
        targeted(if data["title"] == "李四" {
            "li@school.example.edu"
        } else {
            "a@school.example.edu"
        })
    })
    .await;
    assert_eq!(read, 2);
    let ids = candidate_ids(&db).await;
    assert_eq!(ids.len(), 2);
    let (_, profiles) = links(&db, ids[0]).await;
    let profiles: Vec<_> = profiles.iter().map(|(url, _)| url.as_str()).collect();
    let copies = [site.url("/info/1/7.htm"), site.url("/info/2/8.htm")];
    assert_eq!(profiles, copies.each_ref().map(PageUrl::as_str));
    db.close().await;
    dir.close().unwrap();
}

#[tokio::test]
async fn a_teacher_listed_under_several_categories_is_one_unless_told_apart() {
    // Listed under each category, mostly without an address: 张三 is one with himself listed
    // without a page. Two 王五 have addresses of their own, one only on his page, which is read
    // before he is taken as one with anyone; two 赵六 have pages of their own, as two teachers of
    // a name at one college would.
    let (dir, db) = test_fixtures::db::open().await.unwrap();
    let site = Site::new().await;
    site.page(
        "/",
        "<title>首页</title><a href='/prof'>教授</a><a href='/dept'>按系</a>",
    );
    site.page(
        "/prof",
        "<title>教授</title><p><a href='/zhang'>张三</a></p><p>李四 l@school.example.edu</p>\
        <p><a href='/wang'>王五</a></p><p><a href='/zhao1'>赵六</a></p>",
    );
    site.page(
        "/dept",
        "<title>按系</title><p>张三</p><p>李四</p><p>王五 w2@school.example.edu</p>\
        <p><a href='/zhao2'>赵六</a></p>",
    );
    site.page(
        "/zhang",
        "<title>张三</title><p>张三 z@school.example.edu</p><p>研究方向：机器学习</p>",
    );
    site.page(
        "/wang",
        "<title>王五</title><p>王五 w1@school.example.edu</p>",
    );
    site.page(
        "/zhao1",
        "<title>赵六</title><p>赵六</p><p>化学系 研究方向：催化</p>",
    );
    site.page(
        "/zhao2",
        "<title>赵六</title><p>赵六</p><p>物理系 研究方向：光学</p>",
    );
    let mut provider = Provider::start().await.unwrap();
    let id = config(&db, &provider.address).await;
    let model = tokio::spawn(async move {
        let link = regex::Regex::new(r"\[([^\]]+)\]\((\d+)\)").unwrap();
        while let Some(r) = provider.requests.recv().await {
            let system = r.body["messages"][0]["content"].as_str().unwrap();
            let data: Value =
                serde_json::from_str(r.body["messages"][1]["content"].as_str().unwrap()).unwrap();
            let answer = if system.starts_with("你整理学院") {
                let next: Vec<_> = offered(&data).into_iter().map(|(_, n)| n).collect();
                if data["title"] == "首页" {
                    json!({"kind": "entry", "next": next})
                } else {
                    json!({"kind": "list", "next": []})
                }
            } else if system.starts_with("你从教师") {
                let blocks: Vec<String> = (data["blocks"].as_array().unwrap().iter())
                    .map(|b| b.as_str().unwrap().to_owned())
                    .collect();
                let people: Vec<_> = (blocks.iter())
                    .filter_map(|block| {
                        let name = ["张三", "李四", "王五", "赵六"]
                            .into_iter()
                            .find(|name| block.contains(name))?;
                        let profile = link.captures(block).map(|c| c[2].parse().unwrap());
                        let emails: Vec<_> = block
                            .split_whitespace()
                            .filter(|word| word.contains('@'))
                            .collect();
                        Some(listed(name, profile, emails))
                    })
                    .collect();
                json!({ "people": people })
            } else if data["title"] == "张三" {
                targeted("z@school.example.edu")
            } else if data["title"] == "王五" {
                targeted("w1@school.example.edu")
            } else {
                json!({"relation": "matched", "emails": [], "research": "催化", "names": []})
            };
            r.respond.send(Response::answer(&answer.to_string())).ok();
        }
    });
    let crawl = queue::start(&db, &[site.url("/")], id, 1).await.unwrap();
    let (tx, mut events) = mpsc::unbounded_channel();
    let task = tokio::spawn(run_with_pages(
        db.clone(),
        net::Net::new(vec![]).unwrap(),
        Arc::new(Pages::local(site.port)),
        Crawler::default(),
        Turns::default(),
        move || {
            tx.send(()).ok();
        },
        |_| {},
    ));
    changed_until(&db, crawl, &mut events, "done").await;
    task.abort();
    model.abort();
    let candidates: Vec<(String, String)> =
        sqlx::query_as("SELECT name, emails FROM candidate ORDER BY name, emails")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(
        candidates,
        [
            ("张三", r#"["z@school.example.edu"]"#),
            ("李四", r#"["l@school.example.edu"]"#),
            ("王五", r#"["w1@school.example.edu"]"#),
            ("王五", r#"["w2@school.example.edu"]"#),
            ("赵六", "[]"),
            ("赵六", "[]"),
        ]
        .map(|(name, emails)| (name.to_owned(), emails.to_owned()))
    );
    db.close().await;
    dir.close().unwrap();
}

#[tokio::test]
async fn a_teacher_s_page_is_read_when_the_one_known_before_was_not() {
    // The first list gives his address, but its link to his page is broken.
    let (dir, db) = test_fixtures::db::open().await.unwrap();
    let site = columns().await;
    site.page(
        "/",
        "<title>教授</title><p><a href='/info/1/7.htm'>张三</a> a@school.example.edu</p>\
        <a href='/boshi'>博士生导师</a>",
    );
    site.page(
        "/info/2/8.htm",
        "<title>张三</title><p>张三 a@school.example.edu</p><p>研究方向：机器学习</p>",
    );
    let read = read_for_zhang(&db, &site, vec!["a@school.example.edu"], |_| {
        targeted("a@school.example.edu")
    })
    .await;
    assert_eq!(read, 1);
    let research: Vec<Option<String>> = sqlx::query_scalar("SELECT research FROM candidate")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(research, [Some("机器学习".to_owned())]);
    db.close().await;
    dir.close().unwrap();
}

#[tokio::test]
async fn a_page_showing_the_address_of_a_teacher_of_the_same_name_is_read() {
    let (dir, db) = test_fixtures::db::open().await.unwrap();
    let site = columns().await;
    site.page(
        "/info/1/7.htm",
        "<title>张三</title><p>张三</p><p>化学系 教授</p><p>a@school.example.edu</p>\
        <p>办公室：化学楼 101</p><p>研究方向：催化</p>",
    );
    site.page(
        "/info/2/8.htm",
        "<title>张三</title><p>张三</p><p>物理系 副教授</p><p>b@school.example.edu</p>\
        <p>办公室：物理楼 202</p><p>研究方向：凝聚态物理</p><p>化学系 张三 a@school.example.edu</p>",
    );
    let read = read_for_zhang(&db, &site, vec![], |data| {
        targeted(if data.to_string().contains("物理系") {
            "b@school.example.edu"
        } else {
            "a@school.example.edu"
        })
    })
    .await;
    assert_eq!(read, 2);
    let emails: Vec<String> = sqlx::query_scalar("SELECT emails FROM candidate ORDER BY id")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(
        emails,
        [r#"["a@school.example.edu"]"#, r#"["b@school.example.edu"]"#]
    );
    db.close().await;
    dir.close().unwrap();
}

#[tokio::test]
async fn a_page_giving_its_own_address_where_one_read_before_gave_another_is_read() {
    // Two teachers named 张三 of two departments, each page linking its own address: in its footer,
    // both pages writing their group's contacts, which are the two of them; beside a menu linking
    // both; beside a menu giving both in lines just like its own; in a block writing both; giving
    // both in the same order, the two swapping whose is 本人's and whose 助理's, said on the line of
    // each address, in a block of its own before or after it, in the same words or in others, or in
    // a heading over a label both addresses follow, the second in a footer of the page's article.
    for page in [
        "<p>课题组联系人 a@school.example.edu b@school.example.edu</p>\
        <div class='footer'><a href='mailto:OWN@school.example.edu'>联系本人</a></div>",
        "<p><a href='mailto:OWN@school.example.edu'>联系本人</a></p>\
        <nav><a href='mailto:a@school.example.edu'>化学系 张三</a>\
        <a href='mailto:b@school.example.edu'>物理系 张三</a></nav>",
        "<p><a href='mailto:OWN@school.example.edu'>联系本人</a></p>\
        <nav><p>化学系 张三</p><p><a href='mailto:a@school.example.edu'>联系本人</a></p>\
        <p>物理系 张三</p><p><a href='mailto:b@school.example.edu'>联系本人</a></p></nav>",
        "<p><a href='mailto:OWN@school.example.edu'>联系本人</a> 课题组联系人 \
        化学系 a@school.example.edu 物理系 b@school.example.edu</p>",
        "<p><a href='mailto:a@school.example.edu'>联系A a@school.example.edu</a> \
        <a href='mailto:b@school.example.edu'>联系B b@school.example.edu</a></p>",
        "<dl><dt>联系A</dt><dd>a@school.example.edu</dd>\
        <dt>联系B</dt><dd>b@school.example.edu</dd></dl>",
        "<p>a@school.example.edu</p><p>联系A</p><p>b@school.example.edu</p><p>联系B</p>",
        "<dl><dt>甲</dt><dd>a@school.example.edu</dd><dt>乙</dt><dd>b@school.example.edu</dd></dl>",
        "<section><h3>联系A</h3><p>电子邮件</p><p>a@school.example.edu</p></section>\
        <section><h3>联系B</h3><p>电子邮件</p><p>b@school.example.edu</p></section>",
        "<article><section><h3>联系A</h3><p>电子邮件</p><p>a@school.example.edu</p></section>\
        <footer><h3>联系B</h3><p>电子邮件</p><p>b@school.example.edu</p></footer></article>",
    ] {
        let (dir, db) = test_fixtures::db::open().await.unwrap();
        let site = columns().await;
        for (path, department, own, [a, b], [first, second]) in [
            (
                "/info/1/7.htm",
                "化学系",
                "a",
                ["本人", "助理"],
                ["本人邮箱", "助理邮箱"],
            ),
            (
                "/info/2/8.htm",
                "物理系",
                "b",
                ["助理", "本人"],
                ["协助联系人", "本人联系方式"],
            ),
        ] {
            let page = (page.replace("OWN", own))
                .replace("联系A", &format!("联系{a}"))
                .replace("联系B", &format!("联系{b}"))
                .replace("甲", first)
                .replace("乙", second);
            site.page(
                path,
                &format!(
                    "<title>张三</title><p>张三 {department}教授</p><p>研究方向：机器学习</p>{page}"
                ),
            );
        }
        let read = read_for_zhang(&db, &site, vec![], |data| {
            targeted(if data.to_string().contains("物理系教授") {
                "b@school.example.edu"
            } else {
                "a@school.example.edu"
            })
        })
        .await;
        assert_eq!(read, 2, "{page}");
        let emails: Vec<String> = sqlx::query_scalar("SELECT emails FROM candidate ORDER BY id")
            .fetch_all(&db)
            .await
            .unwrap();
        assert_eq!(
            emails,
            [r#"["a@school.example.edu"]"#, r#"["b@school.example.edu"]"#],
            "{page}"
        );
        db.close().await;
        dir.close().unwrap();
    }
}

#[tokio::test]
async fn a_copy_writing_an_address_found_only_within_another_before_is_read() {
    // Another version of his page adds li@, which the one read before writes only inside weili@.
    let (dir, db) = test_fixtures::db::open().await.unwrap();
    let site = columns().await;
    let page = "<title>张三</title><p>张三 a@school.example.edu</p>\
        <p>联系人 weili@school.example.edu</p>";
    site.page("/info/1/7.htm", page);
    site.page(
        "/info/2/8.htm",
        &format!("{page}<p>li@school.example.edu</p>"),
    );
    let read = read_for_zhang(&db, &site, vec![], |_| targeted("a@school.example.edu")).await;
    assert_eq!(read, 2);
    db.close().await;
    dir.close().unwrap();
}

#[tokio::test]
async fn a_copy_is_read_for_research_the_one_read_before_did_not_give() {
    let (dir, db) = test_fixtures::db::open().await.unwrap();
    let site = columns().await;
    site.page(
        "/info/1/7.htm",
        "<title>张三</title><p>张三 a@school.example.edu</p>",
    );
    site.page(
        "/info/2/8.htm",
        "<title>张三</title><p>张三 a@school.example.edu</p><p>研究方向：机器学习</p>",
    );
    let read = read_for_zhang(&db, &site, vec![], |data| {
        let mut answer = targeted("a@school.example.edu");
        if !data.to_string().contains("机器学习") {
            answer["research"] = Value::Null;
        }
        answer
    })
    .await;
    assert_eq!(read, 2);
    let research: Vec<Option<String>> = sqlx::query_scalar("SELECT research FROM candidate")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(research, [Some("机器学习".to_owned())]);
    db.close().await;
    dir.close().unwrap();
}

#[tokio::test]
async fn a_site_read_over_http_after_https_failed_reads_its_start_once() {
    // Its pages link back to the start over http, a page a link too far too.
    let (dir, db) = test_fixtures::db::open().await.unwrap();
    let site = Site::new().await;
    site.page("/", "<title>首页</title><a href='/faculty'>师资队伍</a>");
    site.page(
        "/faculty",
        "<title>师资队伍</title><p>张三</p><a href='/'>首页</a><a href='/dept'>数学系</a>",
    );
    site.page("/dept", "<title>数学系</title><a href='/'>首页</a>");
    let mut provider = Provider::start().await.unwrap();
    let id = config(&db, &provider.address).await;
    let model = tokio::spawn(async move {
        while let Some(r) = provider.requests.recv().await {
            let system = r.body["messages"][0]["content"].as_str().unwrap();
            let data: Value =
                serde_json::from_str(r.body["messages"][1]["content"].as_str().unwrap()).unwrap();
            let answer = if system.starts_with("你整理学院") {
                let next: Vec<_> = offered(&data).into_iter().map(|(_, n)| n).collect();
                json!({"kind": "entry", "next": next})
            } else if data["title"] == "师资队伍" {
                json!({"people": [{"n": "张三"}]})
            } else {
                json!({"people": []})
            };
            r.respond.send(Response::answer(&answer.to_string())).ok();
        }
    });
    let https = PageUrl::parse(&format!("https://school.example.edu:{}/", site.port)).unwrap();
    let crawl = queue::start(&db, std::slice::from_ref(&https), id, 1)
        .await
        .unwrap();
    let (tx, mut events) = mpsc::unbounded_channel();
    let task = tokio::spawn(run_with_pages(
        db.clone(),
        net::Net::new(vec![]).unwrap(),
        Arc::new(Pages::local(site.port)),
        Crawler::default(),
        Turns::default(),
        move || {
            tx.send(()).ok();
        },
        |_| {},
    ));
    changed_until(&db, crawl, &mut events, "done").await;
    let urls: Vec<String> = sqlx::query_scalar("SELECT url FROM crawl_page ORDER BY id")
        .fetch_all(&db)
        .await
        .unwrap();
    let pages = ["/faculty", "/dept"].map(|path| site.url(path));
    assert_eq!(urls, [https.as_str(), pages[0].as_str(), pages[1].as_str()]);
    let limits: String = sqlx::query_scalar("SELECT limits FROM crawl WHERE id = ?")
        .bind(crawl)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(limits, "[]");
    task.abort();
    model.abort();
    db.close().await;
    dir.close().unwrap();
}

#[tokio::test]
async fn a_list_routing_takes_for_a_teacher_s_page_is_read_as_a_list() {
    // As sse.sysu.edu.cn's list of off-campus supervisors, whose head shows its first teacher.
    let (dir, db) = test_fixtures::db::open().await.unwrap();
    let site = Site::new().await;
    site.page(
        "/",
        "<title>校外导师</title><p>张三</p><p>研究方向：机器学习</p><p>李四</p>\
        <p>研究方向：数据库</p><a href='/prof'>教授</a>",
    );
    site.page("/prof", "<title>教授</title><p>王五</p>");
    let mut provider = Provider::start().await.unwrap();
    let id = config(&db, &provider.address).await;
    let model = tokio::spawn(async move {
        while let Some(r) = provider.requests.recv().await {
            let system = r.body["messages"][0]["content"].as_str().unwrap();
            let data: Value =
                serde_json::from_str(r.body["messages"][1]["content"].as_str().unwrap()).unwrap();
            let first = data["title"] == "校外导师";
            let answer = if system.starts_with("你整理学院") {
                let next: Vec<_> = (offered(&data).into_iter())
                    .filter(|(text, _)| text == "教授")
                    .map(|(_, n)| n)
                    .collect();
                json!({"kind": if first { "profile" } else { "list" }, "next": next})
            } else if system.starts_with("你整理一位") {
                json!({
                    "name": null,
                    "aliases": [],
                    "emails": [],
                    "affiliations": [],
                    "homepages": [],
                    "research": null,
                    "papers": []
                })
            } else {
                let names = if first {
                    &["张三", "李四"][..]
                } else {
                    &["王五"]
                };
                json!({"people": names.iter().map(|n| json!({"n": n})).collect::<Vec<_>>()})
            };
            r.respond.send(Response::answer(&answer.to_string())).ok();
        }
    });
    let crawl = queue::start(&db, &[site.url("/")], id, 1).await.unwrap();
    let (tx, mut events) = mpsc::unbounded_channel();
    let task = tokio::spawn(run_with_pages(
        db.clone(),
        net::Net::new(vec![]).unwrap(),
        Arc::new(Pages::local(site.port)),
        Crawler::default(),
        Turns::default(),
        move || {
            tx.send(()).ok();
        },
        |_| {},
    ));
    changed_until(&db, crawl, &mut events, "done").await;
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM candidate ORDER BY id")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(names, ["张三", "李四", "王五"]);
    task.abort();
    model.abort();
    db.close().await;
    dir.close().unwrap();
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn later_profile_facts_need_review_and_keep_the_user_approved_identity(db: SqlitePool) {
    let url = PageUrl::parse("https://school.example.edu/profile").unwrap();
    let config = config(&db, "http://127.0.0.1:1").await;
    let crawl = queue::start(&db, std::slice::from_ref(&url), config, 1)
        .await
        .unwrap();
    let page = queue::next(&db).await.unwrap().unwrap();
    let fields = MentorFields {
        name: "张三".into(),
        ..Default::default()
    };
    let person = |papers| Person {
        fields: fields.clone(),
        papers,
        profile: Some(url.clone()),
        recognized: vec![],
    };
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await.unwrap();
    review::save(
        &mut tx,
        crawl,
        review::Source::Profile {
            id: page.id,
            own: true,
            target: None,
        },
        url.as_str(),
        person(vec![]),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let id = sqlx::query_scalar::<_, i64>("SELECT id FROM candidate")
        .fetch_one(&db)
        .await
        .unwrap();
    let input = ReviewInput {
        ids: vec![id],
        decisions: Decisions::default(),
    };
    review::accept(&db, &input, 2).await.unwrap();
    let mentor = review::detail(&db, id)
        .await
        .unwrap()
        .unwrap()
        .candidate
        .mentor_id
        .unwrap();
    mentor::update(
        &db,
        mentor,
        &MentorFields {
            name: "用户修改的姓名".into(),
            ..Default::default()
        },
        3,
    )
    .await
    .unwrap();
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await.unwrap();
    review::save(
        &mut tx,
        crawl,
        review::Source::Profile {
            id: page.id,
            own: true,
            target: None,
        },
        url.as_str(),
        person(vec!["新补到的论文".into()]),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    queue::delete(&db, crawl).await.unwrap();
    let detail = review::detail(&db, id).await.unwrap().unwrap();
    assert_eq!(detail.candidate.crawl_id, None);
    assert_eq!(detail.candidate.review, "pending");
    assert_eq!(detail.candidate.mentor_id, Some(mentor));
    let preview = review::preview(&db, &input).await.unwrap();
    assert_eq!(preview.created, 0);
    assert_eq!(preview.updated, 1);
    review::accept(&db, &input, 4).await.unwrap();
    let teacher = mentor::get(&db, mentor).await.unwrap().unwrap();
    assert_eq!(teacher.name, "用户修改的姓名");
    assert_eq!(teacher.papers, vec!["新补到的论文"]);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mentor")
            .fetch_one(&db)
            .await
            .unwrap(),
        1
    );
    assert_eq!(review::accept(&db, &input, 5).await.unwrap().accepted, 0);
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn combining_duplicate_pages_keeps_an_already_accepted_candidate(db: SqlitePool) {
    let url = PageUrl::parse("https://school.example.edu/one").unwrap();
    let two = url.join("/two").unwrap();
    let config = config(&db, "http://127.0.0.1:1").await;
    let crawl = queue::start(&db, std::slice::from_ref(&url), config, 1)
        .await
        .unwrap();
    let page = queue::next(&db).await.unwrap().unwrap();
    let person = |url: PageUrl, emails: Vec<String>| Person {
        fields: MentorFields {
            name: "张三".into(),
            emails,
            ..Default::default()
        },
        papers: vec![],
        profile: Some(url),
        recognized: vec![],
    };
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await.unwrap();
    for u in [&url, &two] {
        review::save(
            &mut tx,
            crawl,
            review::Source::Profile {
                id: page.id,
                own: true,
                target: None,
            },
            u.as_str(),
            person(u.clone(), vec![]),
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    let accepted = sqlx::query_scalar::<_, i64>("SELECT max(id) FROM candidate")
        .fetch_one(&db)
        .await
        .unwrap();
    let input = ReviewInput {
        ids: vec![accepted],
        decisions: Decisions::default(),
    };
    review::accept(&db, &input, 2).await.unwrap();
    let mentor = review::detail(&db, accepted)
        .await
        .unwrap()
        .unwrap()
        .candidate
        .mentor_id
        .unwrap();
    mentor::update(
        &db,
        mentor,
        &MentorFields {
            name: "修改后的姓名".into(),
            ..Default::default()
        },
        3,
    )
    .await
    .unwrap();
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await.unwrap();
    for u in [&url, &two] {
        review::save(
            &mut tx,
            crawl,
            review::Source::Profile {
                id: page.id,
                own: true,
                target: None,
            },
            u.as_str(),
            person(u.clone(), vec!["zhang@example.edu".into()]),
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM candidate")
            .fetch_one(&db)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        review::detail(&db, accepted)
            .await
            .unwrap()
            .unwrap()
            .candidate
            .mentor_id,
        Some(mentor)
    );
    review::accept(&db, &input, 4).await.unwrap();
    assert_eq!(
        mentor::get(&db, mentor).await.unwrap().unwrap().emails,
        vec!["zhang@example.edu"]
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mentor")
            .fetch_one(&db)
            .await
            .unwrap(),
        1
    );
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn retrying_one_failed_page_after_cancel_does_not_restart_other_planned_pages(
    db: SqlitePool,
) {
    let first = PageUrl::parse("https://school.example.edu/one").unwrap();
    let second = first.join("/two").unwrap();
    let model = config(&db, "http://127.0.0.1:1").await;
    let id = queue::start(&db, &[first, second], model, 1).await.unwrap();
    let failed = queue::next(&db).await.unwrap().unwrap();
    save(&db, &failed, &Outcome::Failed(Failure::new("读取超时")))
        .await
        .unwrap();
    queue::control(&db, id, "cancel", 2).await.unwrap();
    assert_eq!(queue::retry(&db, id, Some(failed.id), 3).await.unwrap(), 1);
    let retry = queue::next(&db).await.unwrap().unwrap();
    assert_eq!(retry.id, failed.id);
    save(&db, &retry, &Outcome::Failed(Failure::new("仍然超时")))
        .await
        .unwrap();
    assert!(queue::next(&db).await.unwrap().is_none());
}

async fn observation(
    db: &SqlitePool,
    c: &Claim,
    site: &Site,
    path: &str,
    kind: PageKind,
    fields: MentorFields,
) {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await.unwrap();
    if kind == PageKind::Profile {
        sqlx::query("UPDATE crawl_page SET work_state='done' WHERE crawl_id=? AND url=?")
            .bind(c.crawl)
            .bind(site.url(path).as_str())
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    review::save(
        &mut tx,
        c.crawl,
        if kind == PageKind::List {
            review::Source::List { id: c.id }
        } else {
            review::Source::Profile {
                id: c.id,
                own: true,
                target: None,
            }
        },
        site.url(path).as_str(),
        Person {
            fields,
            papers: vec![],
            profile: Some(site.url(path)),
            recognized: vec![],
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
}
fn observed(name: &str, emails: &[&str], research: Option<&str>) -> MentorFields {
    MentorFields {
        name: name.into(),
        emails: emails.iter().map(|s| (*s).into()).collect(),
        research: research.map(Into::into),
        ..Default::default()
    }
}
/// The links `links` gives for pages at `urls` proved not to be the candidate's: each over either
/// scheme.
fn proved_not(urls: &[&str]) -> Vec<(String, bool)> {
    let mut links: Vec<_> = (crate::url::both_schemes(urls).into_iter())
        .map(|url| (url, true))
        .collect();
    links.sort();
    links
}
async fn links(db: &SqlitePool, candidate: i64) -> (Option<String>, Vec<(String, bool)>) {
    let profile = sqlx::query_scalar("SELECT profile_url FROM candidate WHERE id = ?")
        .bind(candidate)
        .fetch_one(db)
        .await
        .unwrap();
    let links = sqlx::query_as(
        r#"SELECT url, mismatched_at IS NOT NULL
        FROM candidate_profile
        WHERE candidate_id = ?
        ORDER BY url"#,
    )
    .bind(candidate)
    .fetch_all(db)
    .await
    .unwrap();
    (profile, links)
}
async fn candidate_ids(db: &SqlitePool) -> Vec<i64> {
    sqlx::query_scalar("SELECT id FROM candidate ORDER BY id")
        .fetch_all(db)
        .await
        .unwrap()
}
async fn review_fixture(db: &SqlitePool) -> (Site, Claim) {
    let site = Site::new().await;
    let model = config(db, "http://127.0.0.1:1").await;
    queue::start(db, &[site.url("/")], model, 1).await.unwrap();
    let claim = queue::next(db).await.unwrap().unwrap();
    (site, claim)
}
async fn profile_fixture(db: &SqlitePool, site: &Site) -> (Claim, tokio::task::JoinHandle<()>) {
    let mut provider = Provider::start().await.unwrap();
    let model = config(db, &provider.address).await;
    queue::start(db, &[site.url("/")], model, 1).await.unwrap();
    let root = queue::next(db).await.unwrap().unwrap();
    save(db, &root, &Outcome::Failed(Failure::new("fixture root")))
        .await
        .unwrap();
    // The model knows a person's page by its title, and finds their line in a directory.
    let server = tokio::spawn(async move {
        while let Some(r) = provider.requests.recv().await {
            let data: Value =
                serde_json::from_str(r.body["messages"][1]["content"].as_str().unwrap()).unwrap();
            let title = mentor::intake::comparable(data["title"].as_str().unwrap());
            let target = &data["target"];
            let name = target["name"].as_str().unwrap();
            let mut answer = targeted(if name == "李四" {
                "li@example.edu"
            } else {
                "zhang@example.edu"
            });
            let theirs = std::iter::once(&target["name"])
                .chain(target["aliases"].as_array().unwrap())
                .any(|n| mentor::intake::comparable(n.as_str().unwrap()) == title);
            if !theirs {
                answer["relation"] = json!("mismatched");
                if title != "教师名单" {
                    answer["emails"] = json!([]);
                }
            }
            r.respond.send(Response::answer(&answer.to_string())).ok();
        }
    });
    (root, server)
}
async fn read_claim(db: &SqlitePool, site: &Site, claim: &Claim) -> Outcome {
    process(
        db,
        &net::Net::new(vec![]).unwrap(),
        &Pages::local(site.port),
        &Turns::default(),
        claim,
        &Cooldown::new(Arc::default()),
    )
    .await
    .unwrap()
}
async fn process_claim(db: &SqlitePool, site: &Site, claim: &Claim) -> Outcome {
    let outcome = read_claim(db, site, claim).await;
    assert!(save(db, claim, &outcome).await.unwrap());
    outcome
}
/// A list line, which gives its profile link as a homepage too.
fn list_line(site: &Site, path: &str, name: &str, email: &str) -> MentorFields {
    MentorFields {
        homepages: vec![site.url(path).as_str().into()],
        ..observed(name, &[email], None)
    }
}
/// A list line without a profile link.
async fn bare_line(db: &SqlitePool, c: &Claim, site: &Site, name: &str, emails: &[&str]) {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await.unwrap();
    review::save(
        &mut tx,
        c.crawl,
        review::Source::List { id: c.id },
        site.url("/").as_str(),
        Person {
            fields: observed(name, emails, None),
            papers: vec![],
            profile: None,
            recognized: vec![],
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn b3_redirect_to_another_person_must_not_merge(db: SqlitePool) {
    let site = Site::new().await;
    site.redirect("/old-zhang", "/li");
    site.page("/li", "<title>李四</title><p>李四 li@example.edu</p>");
    let (root, server) = profile_fixture(&db, &site).await;
    for (path, name, email) in [
        ("/old-zhang", "张三", "zhang@example.edu"),
        ("/li", "李四", "li@example.edu"),
    ] {
        observation(
            &db,
            &root,
            &site,
            path,
            PageKind::List,
            list_line(&site, path, name, email),
        )
        .await;
    }
    let ids = candidate_ids(&db).await;
    let before = serde_json::to_value(review::detail(&db, ids[0]).await.unwrap()).unwrap();
    let claim = queue::next(&db).await.unwrap().unwrap();
    assert!(matches!(
        process_claim(&db, &site, &claim).await,
        Outcome::Mismatched { .. }
    ));
    assert_eq!(candidate_ids(&db).await, ids);
    // The link is no profile or homepage of his any more, and identifies nobody.
    let mut expected = before;
    expected["fields"]["homepages"] = json!([]);
    assert_eq!(
        serde_json::to_value(review::detail(&db, ids[0]).await.unwrap()).unwrap(),
        expected
    );
    // Nor is 李四's page it ended at his.
    let old_zhang = site.url("/old-zhang").as_str().to_owned();
    assert_eq!(
        links(&db, ids[0]).await,
        (None, proved_not(&[&old_zhang, site.url("/li").as_str()]))
    );
    let detail = queue::detail(&db, root.crawl, 1, false)
        .await
        .unwrap()
        .unwrap();
    let page = detail.pages.iter().find(|p| p.id == claim.id).unwrap();
    assert_eq!(page.status, PageStatus::Failed);
    assert!(page.error.as_deref().unwrap().message.contains("张三"));
    let li = queue::next(&db).await.unwrap().unwrap();
    assert_eq!(li.url, site.url("/li"));
    process_claim(&db, &site, &li).await;
    assert_eq!(
        review::accept(
            &db,
            &ReviewInput {
                ids: ids.clone(),
                decisions: Decisions::default()
            },
            2
        )
        .await
        .unwrap()
        .accepted,
        2
    );
    for (id, name, email) in [
        (ids[0], "张三", "zhang@example.edu"),
        (ids[1], "李四", "li@example.edu"),
    ] {
        let candidate = review::detail(&db, id).await.unwrap().unwrap().candidate;
        let mentor = mentor::get(&db, candidate.mentor_id.unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(mentor.name, name);
        assert_eq!(mentor.emails, vec![email]);
        assert!(mentor.aliases.is_empty());
    }
    site.redirect("/old-zhang", "/zhang");
    site.page("/zhang", "<title>张三</title><p>张三 zhang@example.edu</p>");
    assert_eq!(
        queue::retry(&db, root.crawl, Some(claim.id), 3)
            .await
            .unwrap(),
        1
    );
    let retry = queue::next(&db).await.unwrap().unwrap();
    assert_eq!(retry.id, claim.id);
    process_claim(&db, &site, &retry).await;
    assert_eq!(candidate_ids(&db).await, ids);
    let detail = queue::detail(&db, root.crawl, 1, false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        detail
            .pages
            .iter()
            .find(|p| p.id == claim.id)
            .unwrap()
            .status,
        PageStatus::Success
    );
    // Read for him again, the page proves his own, and the link is his again over either scheme;
    // 李四's page stays not his.
    let mut expected = proved_not(&[site.url("/li").as_str()]);
    expected.extend(
        crate::url::both_schemes(&[&old_zhang])
            .into_iter()
            .chain([site.url("/zhang").as_str().to_owned()])
            .map(|url| (url, false)),
    );
    expected.sort();
    assert_eq!(links(&db, ids[0]).await, (Some(old_zhang), expected));
    server.abort();
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_link_found_not_to_be_theirs_is_read_again_for_whom_another_list_gives_it(
    db: SqlitePool,
) {
    let site = Site::new().await;
    site.page("/li", "<title>李四</title><p>李四 li@example.edu</p>");
    let (root, server) = profile_fixture(&db, &site).await;
    let link = site.url("/li").as_str().to_owned();
    observation(
        &db,
        &root,
        &site,
        "/li",
        PageKind::List,
        list_line(&site, "/li", "张三", "zhang@example.edu"),
    )
    .await;
    let zhang = candidate_ids(&db).await[0];
    let claim = queue::next(&db).await.unwrap().unwrap();
    assert!(matches!(
        process_claim(&db, &site, &claim).await,
        Outcome::Mismatched { .. }
    ));
    // The owner's own list line does not merge into the one the link proved not to be.
    let li_line = list_line(&site, "/li", "李四", "li@example.edu");
    observation(&db, &root, &site, "/li", PageKind::List, li_line.clone()).await;
    let ids = candidate_ids(&db).await;
    assert_eq!(ids.len(), 2);
    let li = ids[1];
    let again = queue::next(&db).await.unwrap().unwrap();
    assert_eq!(again.id, claim.id);
    assert!(matches!(
        process_claim(&db, &site, &again).await,
        Outcome::Done(_)
    ));
    assert!(queue::next(&db).await.unwrap().is_none());
    assert_eq!(
        links(&db, li).await,
        (Some(link.clone()), vec![(link.clone(), false)])
    );
    assert_eq!(links(&db, zhang).await, (None, proved_not(&[&link])));
    let li = review::detail(&db, li).await.unwrap().unwrap().candidate;
    assert_eq!(li.fields.emails, ["li@example.edu"]);
    assert_eq!(li.fields.homepages, [link]);
    let zhang = review::detail(&db, zhang).await.unwrap().unwrap().candidate;
    assert_eq!(zhang.fields.emails, ["zhang@example.edu"]);
    assert!(zhang.fields.homepages.is_empty());
    // Read for him already, the page is not read again when a list gives him the link again.
    observation(&db, &root, &site, "/li", PageKind::List, li_line).await;
    assert!(queue::next(&db).await.unwrap().is_none());
    server.abort();
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_link_another_list_gave_someone_of_another_name_is_not_theirs(db: SqlitePool) {
    // Two lists giving their group's page to two teachers, as one list may give it to several.
    let site = Site::new().await;
    let (root, server) = profile_fixture(&db, &site).await;
    for (name, email) in [("张三", "zhang@example.edu"), ("李四", "li@example.edu")] {
        let line = list_line(&site, "/group", name, email);
        observation(&db, &root, &site, "/group", PageKind::List, line).await;
    }
    let ids = candidate_ids(&db).await;
    assert_eq!(ids.len(), 2);
    let link = site.url("/group").as_str().to_owned();
    assert_eq!(
        links(&db, ids[0]).await,
        (Some(link.clone()), vec![(link, false)])
    );
    assert_eq!(links(&db, ids[1]).await, (None, vec![]));
    let li = review::detail(&db, ids[1])
        .await
        .unwrap()
        .unwrap()
        .candidate;
    assert_eq!(li.fields.emails, ["li@example.edu"]);
    assert!(li.fields.homepages.is_empty());
    server.abort();
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_page_counts_for_whom_its_candidate_merges_into_while_it_is_read(db: SqlitePool) {
    let site = Site::new().await;
    site.page("/zhang", "<title>张三</title><p>张三 zhang@example.edu</p>");
    site.page("/wang", "<title>李四</title><p>李四 li@example.edu</p>");
    let (root, server) = profile_fixture(&db, &site).await;
    bare_line(&db, &root, &site, "张三", &["a@example.edu"]).await;
    bare_line(&db, &root, &site, "王五", &["b@example.edu"]).await;
    for (path, name, email) in [
        ("/zhang", "张三", "c@example.edu"),
        ("/wang", "王五", "w@example.edu"),
    ] {
        observation(
            &db,
            &root,
            &site,
            path,
            PageKind::List,
            observed(name, &[email], None),
        )
        .await;
    }
    let ids = candidate_ids(&db).await;
    let zhang = queue::next(&db).await.unwrap().unwrap();
    let wang = queue::next(&db).await.unwrap().unwrap();
    assert_eq!(zhang.url, site.url("/zhang"));
    let read = read_claim(&db, &site, &zhang).await;
    let misread = read_claim(&db, &site, &wang).await;
    assert!(matches!(misread, Outcome::Mismatched { .. }));
    // Another list shows each line to be one read before.
    bare_line(
        &db,
        &root,
        &site,
        "张三",
        &["a@example.edu", "c@example.edu"],
    )
    .await;
    bare_line(
        &db,
        &root,
        &site,
        "王五",
        &["b@example.edu", "w@example.edu"],
    )
    .await;
    assert!(save(&db, &zhang, &read).await.unwrap());
    assert!(save(&db, &wang, &misread).await.unwrap());
    assert_eq!(candidate_ids(&db).await, ids[..2]);
    let zhang_link = site.url("/zhang").as_str().to_owned();
    assert_eq!(
        links(&db, ids[0]).await,
        (Some(zhang_link.clone()), vec![(zhang_link, false)])
    );
    let a = review::detail(&db, ids[0])
        .await
        .unwrap()
        .unwrap()
        .candidate;
    assert!(a.fields.emails.contains(&"zhang@example.edu".to_owned()));
    let wang_link = site.url("/wang").as_str().to_owned();
    assert_eq!(links(&db, ids[1]).await, (None, proved_not(&[&wang_link])));
    let b = review::detail(&db, ids[1])
        .await
        .unwrap()
        .unwrap()
        .candidate;
    assert_eq!(b.fields.emails, ["b@example.edu", "w@example.edu"]);
    server.abort();
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn merging_in_a_link_proved_not_theirs_takes_it_from_the_profile(db: SqlitePool) {
    let (site, root) = review_fixture(&db).await;
    for (path, email) in [("/u", "a@example.edu"), ("/v", "b@example.edu")] {
        observation(
            &db,
            &root,
            &site,
            path,
            PageKind::List,
            observed("张三", &[email], None),
        )
        .await;
    }
    let ids = candidate_ids(&db).await;
    let url = |path: &str| site.url(path).as_str().to_owned();
    // The second's page at the first's profile proved not to be theirs, and a third link neither's.
    for (candidate, path, at) in [
        (ids[1], "/u", 300),
        (ids[0], "/w", 100),
        (ids[1], "/w", 200),
    ] {
        sqlx::query(
            r#"INSERT INTO candidate_profile (candidate_id, crawl_id, url, mismatched_at)
            VALUES (?, ?, ?, ?)"#,
        )
        .bind(candidate)
        .bind(root.crawl)
        .bind(url(path))
        .bind(at)
        .execute(&db)
        .await
        .unwrap();
    }
    bare_line(
        &db,
        &root,
        &site,
        "张三",
        &["a@example.edu", "b@example.edu"],
    )
    .await;
    assert_eq!(candidate_ids(&db).await, ids[..1]);
    assert_eq!(
        links(&db, ids[0]).await,
        (
            Some(url("/v")),
            vec![(url("/u"), true), (url("/v"), false), (url("/w"), true)]
        )
    );
    let latest: i64 =
        sqlx::query_scalar("SELECT mismatched_at FROM candidate_profile WHERE url = ?")
            .bind(url("/w"))
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(latest, 200);
    // The link is free for whom another list gives it.
    observation(
        &db,
        &root,
        &site,
        "/u",
        PageKind::List,
        observed("李四", &["li@example.edu"], None),
    )
    .await;
    let li = candidate_ids(&db).await[1];
    assert_eq!(
        links(&db, li).await,
        (Some(url("/u")), vec![(url("/u"), false)])
    );
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_page_found_as_a_profile_by_itself_is_read_again_for_whom_a_list_gives_it(
    db: SqlitePool,
) {
    let (site, root) = review_fixture(&db).await;
    // The start page itself proved to be 张三's profile, read for nobody in particular.
    sqlx::query(
        r#"UPDATE crawl_page
        SET kind = 'profile', work_state = 'done', status = 'success', error = NULL, text = ''
        WHERE id = ?"#,
    )
    .bind(root.id)
    .execute(&db)
    .await
    .unwrap();
    let link = site.url("/");
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await.unwrap();
    review::save(
        &mut tx,
        root.crawl,
        review::Source::Profile {
            id: root.id,
            own: true,
            target: None,
        },
        link.as_str(),
        Person {
            fields: observed("张三", &["zhang@example.edu"], None),
            papers: vec![],
            profile: Some(link.clone()),
            recognized: vec![],
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let zhang = candidate_ids(&db).await[0];
    // Enriching him later found it not to be his.
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await.unwrap();
    review::mismatched(&mut tx, zhang, &[link.as_str()], 2)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    observation(
        &db,
        &root,
        &site,
        "/",
        PageKind::List,
        observed("李四", &["li@example.edu"], None),
    )
    .await;
    let li = candidate_ids(&db).await[1];
    let again = queue::next(&db).await.unwrap().unwrap();
    assert_eq!(again.id, root.id);
    let target: Option<i64> = sqlx::query_scalar("SELECT target_id FROM crawl_page WHERE id = ?")
        .bind(root.id)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(target, Some(li));
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_profile_reached_from_another_confirmed_candidates_page_is_still_read_for_its_own(
    db: SqlitePool,
) {
    let (site, c) = review_fixture(&db).await;
    for (path, name) in [("/li", "李四"), ("/zhang", "张三")] {
        observation(
            &db,
            &c,
            &site,
            path,
            PageKind::List,
            observed(name, &[], None),
        )
        .await;
        review::accept(
            &db,
            &ReviewInput {
                ids: vec![*candidate_ids(&db).await.last().unwrap()],
                decisions: Decisions {
                    matches: vec![mentor::import::GroupChoice { row: 1, id: None }],
                    ..Default::default()
                },
            },
            2,
        )
        .await
        .unwrap();
    }
    let ids = candidate_ids(&db).await;
    let li_page = queue::next(&db).await.unwrap().unwrap();
    assert_eq!(li_page.url, site.url("/li"));
    // 李四's page ended up at 张三's profile and proved his; the two stay apart.
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await.unwrap();
    review::save(
        &mut tx,
        c.crawl,
        review::Source::Profile {
            id: li_page.id,
            own: true,
            target: Some(ids[0]),
        },
        site.url("/zhang").as_str(),
        Person {
            fields: observed("李四", &[], None),
            papers: vec![],
            profile: Some(site.url("/li")),
            recognized: vec![],
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(candidate_ids(&db).await, ids);
    // 张三's profile is still read, for him.
    let zhang_page = queue::next(&db).await.unwrap().unwrap();
    assert_eq!(zhang_page.url, site.url("/zhang"));
    let target: Option<i64> = sqlx::query_scalar("SELECT target_id FROM crawl_page WHERE id = ?")
        .bind(zhang_page.id)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(target, Some(ids[1]));
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn b3_redirects_to_directory_must_not_merge(db: SqlitePool) {
    let site = Site::new().await;
    site.redirect("/old-zhang", "/directory");
    site.redirect("/old-li", "/directory");
    site.page(
        "/directory",
        "<title>教师名单</title><p>张三 zhang@example.edu</p><p>李四 li@example.edu</p>",
    );
    let (root, server) = profile_fixture(&db, &site).await;
    for (path, name, email) in [
        ("/old-zhang", "张三", "zhang@example.edu"),
        ("/old-li", "李四", "li@example.edu"),
    ] {
        observation(
            &db,
            &root,
            &site,
            path,
            PageKind::List,
            observed(name, &[email], None),
        )
        .await;
    }
    let ids = candidate_ids(&db).await;
    // Each name is on the directory, so each reads its own line; the directory is neither's own
    // page and identifies nobody.
    for _ in 0..2 {
        let claim = queue::next(&db).await.unwrap().unwrap();
        assert!(matches!(
            process_claim(&db, &site, &claim).await,
            Outcome::Done(_)
        ));
    }
    assert_eq!(candidate_ids(&db).await, ids);
    for (id, path, name, email) in [
        (ids[0], "/old-zhang", "张三", "zhang@example.edu"),
        (ids[1], "/old-li", "李四", "li@example.edu"),
    ] {
        let candidate = review::detail(&db, id).await.unwrap().unwrap().candidate;
        assert_eq!(candidate.fields.name, name);
        assert!(candidate.fields.aliases.is_empty());
        assert_eq!(candidate.fields.emails, vec![email]);
        assert!(candidate.fields.homepages.is_empty());
        let identities: Vec<String> =
            sqlx::query_scalar("SELECT url FROM candidate_profile WHERE candidate_id=?")
                .bind(id)
                .fetch_all(&db)
                .await
                .unwrap();
        assert_eq!(identities, vec![site.url(path).as_str()]);
    }
    server.abort();
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn b3_same_request_changed_destination_must_not_merge(db: SqlitePool) {
    let site = Site::new().await;
    site.redirect("/old", "/zhang");
    site.page(
        "/zhang",
        "<title>张三</title><p>张三 zhang@example.edu</p><button>下一页</button>",
    );
    site.page("/li", "<title>李四</title><p>李四 li@example.edu</p>");
    let (root, server) = profile_fixture(&db, &site).await;
    for (path, name, email) in [
        ("/old", "张三", "zhang@example.edu"),
        ("/li", "李四", "li@example.edu"),
    ] {
        observation(
            &db,
            &root,
            &site,
            path,
            PageKind::List,
            observed(name, &[email], None),
        )
        .await;
    }
    let ids = candidate_ids(&db).await;
    let claim = queue::next(&db).await.unwrap().unwrap();
    let Outcome::Done(page) = process_claim(&db, &site, &claim).await else {
        panic!("profile did not finish")
    };
    assert_eq!(
        page.material.needs_browser,
        Some(crate::BrowserReason::Pagination)
    );
    let before = serde_json::to_value(review::detail(&db, ids[0]).await.unwrap()).unwrap();
    site.redirect("/old", "/li");
    assert_eq!(
        queue::retry(&db, root.crawl, Some(claim.id), 2)
            .await
            .unwrap(),
        1
    );
    let retry = queue::next(&db).await.unwrap().unwrap();
    assert_eq!(retry.id, claim.id);
    process_claim(&db, &site, &retry).await;
    assert_eq!(candidate_ids(&db).await, ids);
    assert_eq!(
        serde_json::to_value(review::detail(&db, ids[0]).await.unwrap()).unwrap(),
        before
    );
    let identities: Vec<String> =
        sqlx::query_scalar("SELECT url FROM candidate_profile WHERE candidate_id=? ORDER BY url")
            .bind(ids[0])
            .fetch_all(&db)
            .await
            .unwrap();
    // His page stays his; the link and 李四's page it ends at now are not, over either scheme.
    let mut expected =
        crate::url::both_schemes(&[site.url("/old").as_str(), site.url("/li").as_str()]);
    expected.push(site.url("/zhang").as_str().to_owned());
    expected.sort();
    assert_eq!(identities, expected);
    let detail = queue::detail(&db, root.crawl, 1, false)
        .await
        .unwrap()
        .unwrap();
    let page = detail.pages.iter().find(|p| p.id == retry.id).unwrap();
    assert_eq!(page.status, PageStatus::Failed);
    assert!(page.error.as_deref().unwrap().message.contains("张三"));
    server.abort();
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn b3_five_hop_same_person(db: SqlitePool) {
    let site = Site::new().await;
    for hop in 0..5 {
        site.redirect(&format!("/hop{hop}"), &format!("/hop{}", hop + 1));
    }
    site.page("/hop5", "<title>张三</title><p>张三</p>");
    let (root, server) = profile_fixture(&db, &site).await;
    for path in ["/hop0", "/hop5"] {
        observation(
            &db,
            &root,
            &site,
            path,
            PageKind::List,
            observed("张三", &[], None),
        )
        .await;
    }
    let claim = queue::next(&db).await.unwrap().unwrap();
    process_claim(&db, &site, &claim).await;
    assert_eq!(candidate_ids(&db).await.len(), 1);
    let identities: Vec<String> =
        sqlx::query_scalar("SELECT url FROM candidate_profile ORDER BY url")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(
        identities,
        vec![site.url("/hop0").as_str(), site.url("/hop5").as_str()]
    );
    assert!(queue::next(&db).await.unwrap().is_none());
    assert_eq!(
        *site.visits.lock().unwrap(),
        (0..=5).map(|hop| format!("/hop{hop}")).collect::<Vec<_>>()
    );
    server.abort();
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn b3_redirect_external_is_rejected(db: SqlitePool) {
    let site = Site::new().await;
    site.redirect(
        "/old",
        site.url("/external")
            .as_str()
            .replace("school.example.edu", "outside.example.org")
            .as_str(),
    );
    site.page("/external", "<title>张三</title><p>张三</p>");
    let mut provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let root = queue::next(&db).await.unwrap().unwrap();
    save(&db, &root, &Outcome::Failed(Failure::new("fixture root")))
        .await
        .unwrap();
    observation(
        &db,
        &root,
        &site,
        "/old",
        PageKind::List,
        observed("张三", &[], None),
    )
    .await;
    let claim = queue::next(&db).await.unwrap().unwrap();
    let Outcome::Failed(reason) = process_claim(&db, &site, &claim).await else {
        panic!("external redirect was not rejected")
    };
    assert_eq!(reason, Failure::new("页面超出了这次整理允许访问的网站"));
    assert_eq!(*site.visits.lock().unwrap(), vec!["/old"]);
    assert!(provider.requests.try_recv().is_err());
    let identities: Vec<String> = sqlx::query_scalar("SELECT url FROM candidate_profile")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(identities, vec![site.url("/old").as_str()]);
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn b3_direct_external_profile_same_site_redirect(db: SqlitePool) {
    let site = Site::new().await;
    site.redirect("/external-old", "/external-canonical");
    site.page("/external-canonical", "<title>张三</title><p>张三</p>");
    let (root, server) = profile_fixture(&db, &site).await;
    let old = PageUrl::parse(
        &site
            .url("/external-old")
            .as_str()
            .replace("school.example.edu", "outside.example.org"),
    )
    .unwrap();
    let material = crate::parse(
        &site.url("/"),
        &format!("<a href='{}'>张三</a>", old.as_str()),
    );
    let scope = Scope::new(&site.url("/"));
    let listed = extract::listed(
        &material,
        vec![extract::Listed {
            name: "张三".into(),
            title: None,
            profile: Some(1),
            emails: vec![],
            research: None,
        }],
        &scope,
        None,
    )
    .unwrap();
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await.unwrap();
    for person in listed {
        review::save(
            &mut tx,
            root.crawl,
            review::Source::List { id: root.id },
            material.url.as_str(),
            person,
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    let claim = queue::next(&db).await.unwrap().unwrap();
    assert_eq!(claim.url, old);
    process_claim(&db, &site, &claim).await;
    assert_eq!(candidate_ids(&db).await.len(), 1);
    let identities: Vec<String> =
        sqlx::query_scalar("SELECT url FROM candidate_profile ORDER BY url")
            .fetch_all(&db)
            .await
            .unwrap();
    let canonical = old.join("/external-canonical").unwrap();
    assert_eq!(identities, vec![canonical.as_str(), old.as_str()]);
    server.abort();
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn redirected_profile_matches_normalized_candidate_alias(db: SqlitePool) {
    let site = Site::new().await;
    site.redirect("/old", "/canonical");
    site.page("/canonical", "<title>Zhang San</title><p>Zhang San</p>");
    let (root, server) = profile_fixture(&db, &site).await;
    let mut fields = observed("张三", &[], None);
    fields.aliases = vec![" Ｚｈａｎｇ　 Ｓａｎ ".into()];
    observation(&db, &root, &site, "/old", PageKind::List, fields).await;
    let claim = queue::next(&db).await.unwrap().unwrap();
    process_claim(&db, &site, &claim).await;
    let ids = candidate_ids(&db).await;
    assert_eq!(ids.len(), 1);
    assert_eq!(
        review::detail(&db, ids[0])
            .await
            .unwrap()
            .unwrap()
            .candidate
            .fields
            .name,
        "张三"
    );
    let identities: Vec<String> =
        sqlx::query_scalar("SELECT url FROM candidate_profile ORDER BY url")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(
        identities,
        vec![site.url("/canonical").as_str(), site.url("/old").as_str()]
    );
    server.abort();
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_listed_profile_link_to_another_teacher_fills_nothing(db: SqlitePool) {
    let site = Site::new().await;
    site.page("/profile", "<title>李四</title><p>李四 li@example.edu</p>");
    let (root, server) = profile_fixture(&db, &site).await;
    observation(
        &db,
        &root,
        &site,
        "/profile",
        PageKind::List,
        observed("张三", &[], None),
    )
    .await;
    let ids = candidate_ids(&db).await;
    let before = serde_json::to_value(review::detail(&db, ids[0]).await.unwrap()).unwrap();
    let claim = queue::next(&db).await.unwrap().unwrap();
    process_claim(&db, &site, &claim).await;
    assert_eq!(candidate_ids(&db).await, ids);
    assert_eq!(
        serde_json::to_value(review::detail(&db, ids[0]).await.unwrap()).unwrap(),
        before
    );
    let detail = queue::detail(&db, root.crawl, 1, false)
        .await
        .unwrap()
        .unwrap();
    let page = detail.pages.iter().find(|p| p.id == claim.id).unwrap();
    assert_eq!(page.status, PageStatus::Failed);
    assert!(page.error.as_deref().unwrap().message.contains("张三"));
    server.abort();
}

async fn profile_from_list(
    db: &SqlitePool,
    site: &Site,
    name: &str,
    profile: Value,
) -> (Claim, tokio::task::JoinHandle<()>) {
    site.page(
        "/list",
        &format!("<title>教师名单</title><a href='/profile'>{name}</a>"),
    );
    let mut provider = Provider::start().await.unwrap();
    let model = config(db, &provider.address).await;
    let list = json!({"people":[listed(name, Some(1), vec![])]});
    let server = tokio::spawn(async move {
        while let Some(r) = provider.requests.recv().await {
            let system = r.body["messages"][0]["content"].as_str().unwrap();
            let answer = if system.starts_with("你整理学院") {
                json!({"kind":"list","next":[]})
            } else if system.starts_with("你从教师") {
                list.clone()
            } else {
                profile.clone()
            };
            r.respond.send(Response::answer(&answer.to_string())).ok();
        }
    });
    queue::start(db, &[site.url("/list")], model, 1)
        .await
        .unwrap();
    let list = queue::next(db).await.unwrap().unwrap();
    process_claim(db, site, &list).await;
    let ids = candidate_ids(db).await;
    assert_eq!(ids.len(), 1);
    let before = review::detail(db, ids[0]).await.unwrap().unwrap();
    assert_eq!(before.candidate.fields.name, name);
    assert!(before.candidate.fields.aliases.is_empty());
    assert!(before.candidate.fields.emails.is_empty());
    (queue::next(db).await.unwrap().unwrap(), server)
}

async fn expect_completed_profile(
    db: &SqlitePool,
    claim: &Claim,
    identities: &[&PageUrl],
    name: &str,
    aliases: &[&str],
    email: &str,
) {
    let crawl = queue::detail(db, claim.crawl, 1, false)
        .await
        .unwrap()
        .unwrap();
    let page = crawl.pages.iter().find(|p| p.id == claim.id).unwrap();
    assert_eq!(page.status, PageStatus::Success, "{:?}", page.error);
    let ids = candidate_ids(db).await;
    assert_eq!(ids.len(), 1);
    let detail = review::detail(db, ids[0]).await.unwrap().unwrap();
    assert_eq!(detail.candidate.fields.name, name);
    assert_eq!(detail.candidate.fields.aliases, aliases);
    assert_eq!(detail.candidate.fields.emails, vec![email]);
    assert_eq!(detail.evidence.len(), 2);
    let actual: Vec<String> = sqlx::query_scalar("SELECT url FROM candidate_profile ORDER BY url")
        .fetch_all(db)
        .await
        .unwrap();
    let mut expected: Vec<_> = identities.iter().map(|url| url.as_str()).collect();
    expected.sort_unstable();
    assert_eq!(actual, expected);
    assert!(queue::next(db).await.unwrap().is_none());
}

/// The model's answer for a page of the person, written there also as `alias`.
fn theirs(email: &str, alias: Option<&str>) -> Value {
    let mut answer = targeted(email);
    answer["aliases"] = json!(alias.into_iter().collect::<Vec<_>>());
    answer
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn b4_slash_english_from_real_sample_should_complete(db: SqlitePool) {
    let site = Site::new().await;
    site.redirect("/profile", "/profile/");
    site.page("/profile/", include_str!("../../tests/pages/nju.html"));
    let (claim, server) = profile_from_list(
        &db,
        &site,
        "周志华",
        theirs("zhouzh@nju.edu.cn", Some("Zhi-Hua Zhou")),
    )
    .await;
    process_claim(&db, &site, &claim).await;
    expect_completed_profile(
        &db,
        &claim,
        &[&claim.url, &site.url("/profile/")],
        "周志华",
        &["Zhi-Hua Zhou"],
        "zhouzh@nju.edu.cn",
    )
    .await;
    server.abort();
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn b4_www_profile_with_chinese_alias_should_complete(db: SqlitePool) {
    let site = Site::new().await;
    let destination = PageUrl::parse(
        &site
            .url("/profile")
            .as_str()
            .replace("school.", "www.school."),
    )
    .unwrap();
    site.redirect("/profile", destination.as_str());
    site.page(
        destination.as_str(),
        "<title>Zhang San</title><p>Zhang San（张三） zhang@example.edu</p>",
    );
    let profile = theirs("zhang@example.edu", Some("Zhang San"));
    let (claim, server) = profile_from_list(&db, &site, "张三", profile).await;
    process_claim(&db, &site, &claim).await;
    expect_completed_profile(
        &db,
        &claim,
        &[&claim.url, &destination],
        "张三",
        &["Zhang San"],
        "zhang@example.edu",
    )
    .await;
    server.abort();
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn b4_https_profile_with_chinese_alias_should_complete(db: SqlitePool) {
    let site = Site::start(true).await;
    let destination =
        PageUrl::parse(&site.url("/profile").as_str().replacen("http:", "https:", 1)).unwrap();
    site.redirect("/profile", destination.as_str());
    site.page(
        destination.as_str(),
        "<title>Zhang San</title><p>Zhang San（张三） zhang@example.edu</p>",
    );
    let profile = theirs("zhang@example.edu", Some("Zhang San"));
    let (claim, server) = profile_from_list(&db, &site, "张三", profile).await;
    process_claim(&db, &site, &claim).await;
    expect_completed_profile(
        &db,
        &claim,
        &[&claim.url, &destination],
        "张三",
        &["Zhang San"],
        "zhang@example.edu",
    )
    .await;
    assert!(
        site.visits
            .lock()
            .unwrap()
            .contains(&"https:/profile".into())
    );
    server.abort();
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn b4_slash_profile_with_title_should_complete(db: SqlitePool) {
    let site = Site::new().await;
    site.redirect("/profile", "/profile/");
    site.page(
        "/profile/",
        "<title>张三 教授</title><p>张三 教授 zhang@example.edu</p>",
    );
    let (claim, server) =
        profile_from_list(&db, &site, "张三", theirs("zhang@example.edu", None)).await;
    process_claim(&db, &site, &claim).await;
    expect_completed_profile(
        &db,
        &claim,
        &[&claim.url, &site.url("/profile/")],
        "张三",
        &[],
        "zhang@example.edu",
    )
    .await;
    server.abort();
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn b4_changed_path_matches_page_alias_should_complete(db: SqlitePool) {
    let site = Site::new().await;
    site.redirect("/profile", "/renamed");
    site.page(
        "/renamed",
        "<title>Zhang San</title><p>Zhang San（张三） zhang@example.edu</p>",
    );
    let profile = theirs("zhang@example.edu", Some("　Zhang San　"));
    let (claim, server) = profile_from_list(&db, &site, "张三", profile).await;
    process_claim(&db, &site, &claim).await;
    expect_completed_profile(
        &db,
        &claim,
        &[&claim.url, &site.url("/renamed")],
        "张三",
        &["Zhang San"],
        "zhang@example.edu",
    )
    .await;
    server.abort();
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_redirected_profile_the_model_is_unsure_of_fills_without_identifying(db: SqlitePool) {
    let site = Site::new().await;
    site.redirect("/profile", "/renamed");
    site.page(
        "/renamed",
        "<title>Zhang San</title><p>Zhang San zhang@example.edu</p>",
    );
    let mut profile = theirs("zhang@example.edu", Some("Zhang San"));
    profile["relation"] = json!("uncertain");
    let (claim, server) = profile_from_list(&db, &site, "张三", profile).await;
    process_claim(&db, &site, &claim).await;
    expect_completed_profile(
        &db,
        &claim,
        &[&claim.url],
        "张三",
        &["Zhang San"],
        "zhang@example.edu",
    )
    .await;
    server.abort();
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_profile_the_model_is_unsure_of_gives_only_the_addresses_by_their_name(db: SqlitePool) {
    let site = Site::new().await;
    site.page(
        "/profile",
        "<title>学院通讯录</title><p>张三 zhang@example.edu</p><p>李四 li@example.edu</p>",
    );
    let mut profile = targeted("zhang@example.edu");
    profile["relation"] = json!("uncertain");
    profile["emails"] = json!(["zhang@example.edu", "li@example.edu"]);
    profile["names"] = json!(["张三", "李四"]);
    let (claim, server) = profile_from_list(&db, &site, "张三", profile).await;
    process_claim(&db, &site, &claim).await;
    expect_completed_profile(&db, &claim, &[&claim.url], "张三", &[], "zhang@example.edu").await;
    server.abort();
}

async fn redirected_profiles(db: &SqlitePool, paths: &[&str], in_flight: bool) {
    let site = Site::new().await;
    site.redirect("/old", "/canonical");
    site.page(
        "/canonical",
        "<title>张三</title><p>张三</p><a href='/other'>其他主页</a>",
    );
    let mut provider = Provider::start().await.unwrap();
    let model = config(db, &provider.address).await;
    queue::start(db, &[site.url("/")], model, 1).await.unwrap();
    let root = queue::next(db).await.unwrap().unwrap();
    save(db, &root, &Outcome::Failed(Failure::new("fixture root")))
        .await
        .unwrap();
    for path in paths {
        observation(
            db,
            &root,
            &site,
            path,
            PageKind::List,
            observed("张三", &[], None),
        )
        .await;
    }
    assert_eq!(candidate_ids(db).await.len(), 2);
    let server = tokio::spawn(async move {
        while let Some(r) = provider.requests.recv().await {
            r.respond
                .send(Response::answer(
                    &json!({
                        "relation": "matched",
                        "aliases": [],
                        "emails": [],
                        "affiliations": [],
                        "homepages": [1],
                        "research": null,
                        "papers": [],
                        "names": ["张三"],
                        "next": []
                    })
                    .to_string(),
                ))
                .ok();
        }
    });
    let mut claims = Vec::new();
    if in_flight {
        for _ in 0..2 {
            claims.push(queue::next(db).await.unwrap().unwrap());
        }
        claims.reverse();
    }
    let mut reads = 0;
    while let Some(c) = if in_flight {
        claims.pop()
    } else {
        queue::next(db).await.unwrap()
    } {
        let outcome = process(
            db,
            &net::Net::new(vec![]).unwrap(),
            &Pages::local(site.port),
            &Turns::default(),
            &c,
            &Cooldown::new(Arc::default()),
        )
        .await
        .unwrap();
        let Outcome::Done(p) = &outcome else {
            panic!("profile did not finish")
        };
        assert_eq!(p.material.url, site.url("/canonical"));
        assert!(save(db, &c, &outcome).await.unwrap());
        reads += 1;
    }
    assert_eq!(candidate_ids(db).await.len(), 1);
    let identities: Vec<String> =
        sqlx::query_scalar("SELECT url FROM candidate_profile ORDER BY url")
            .fetch_all(db)
            .await
            .unwrap();
    assert_eq!(
        identities,
        vec![site.url("/canonical").as_str(), site.url("/old").as_str()]
    );
    let expected = if in_flight || paths[0] == "/canonical" {
        2
    } else {
        1
    };
    assert_eq!(reads, expected);
    assert_eq!(
        site.visits
            .lock()
            .unwrap()
            .iter()
            .filter(|p| *p == "/canonical")
            .count(),
        expected
    );
    for path in ["/old", "/canonical"] {
        observation(
            db,
            &root,
            &site,
            path,
            PageKind::List,
            observed("张三", &[], None),
        )
        .await;
    }
    assert_eq!(candidate_ids(db).await.len(), 1);
    assert!(queue::next(db).await.unwrap().is_none());
    server.abort();
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn audit_redirected_profile_alias_does_not_duplicate_a_person(db: SqlitePool) {
    redirected_profiles(&db, &["/old", "/canonical"], false).await;
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn redirected_profile_after_canonical_keeps_one_candidate(db: SqlitePool) {
    redirected_profiles(&db, &["/canonical", "/old"], false).await;
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn redirected_profile_in_flight_result_uses_retained_identity(db: SqlitePool) {
    redirected_profiles(&db, &["/old", "/canonical"], true).await;
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn redirected_profile_preserves_distinct_confirmed_targets(db: SqlitePool) {
    let (site, c) = review_fixture(&db).await;
    for path in ["/old", "/canonical"] {
        observation(
            &db,
            &c,
            &site,
            path,
            PageKind::List,
            observed("张三", &[], None),
        )
        .await;
        review::accept(
            &db,
            &ReviewInput {
                ids: vec![*candidate_ids(&db).await.last().unwrap()],
                decisions: Decisions {
                    matches: vec![mentor::import::GroupChoice { row: 1, id: None }],
                    ..Default::default()
                },
            },
            2,
        )
        .await
        .unwrap();
    }
    let targets: Vec<(i64, Option<i64>)> =
        sqlx::query_as("SELECT id,mentor_id FROM candidate ORDER BY id")
            .fetch_all(&db)
            .await
            .unwrap();
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await.unwrap();
    review::save(
        &mut tx,
        c.crawl,
        review::Source::Profile {
            id: c.id,
            own: true,
            target: None,
        },
        site.url("/canonical").as_str(),
        Person {
            fields: observed("张三", &[], None),
            papers: vec![],
            profile: Some(site.url("/old")),
            recognized: vec![],
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        sqlx::query_as::<_, (i64, Option<i64>)>("SELECT id,mentor_id FROM candidate ORDER BY id")
            .fetch_all(&db)
            .await
            .unwrap(),
        targets
    );
    for id in candidate_ids(&db).await {
        sqlx::query("UPDATE candidate SET review='pending' WHERE id=?")
            .bind(id)
            .execute(&db)
            .await
            .unwrap();
        let input = ReviewInput {
            ids: vec![id],
            decisions: Decisions::default(),
        };
        assert_eq!(review::preview(&db, &input).await.unwrap().errors.len(), 1);
        assert_eq!(review::accept(&db, &input, 3).await.unwrap().accepted, 0);
    }
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn crawl_options_include_all_runs_in_start_time_order(db: SqlitePool) {
    for at in (1..=52).rev() {
        sqlx::query(
            "INSERT INTO crawl(kind,start_urls,state,created_at,finished_at) VALUES('site',?1,'done',?2,?2)",
        )
        .bind(json!([format!("https://school.example.edu/{at}")]).to_string())
        .bind(at)
        .execute(&db)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO crawl(kind,state,created_at,finished_at) VALUES('agent','done',53,53)",
    )
    .execute(&db)
    .await
    .unwrap();
    let options = queue::options(&db).await.unwrap();
    assert_eq!(options.len(), 53);
    assert!(options[0].url.is_none());
    assert_eq!(
        options.iter().map(|c| c.created_at).collect::<Vec<_>>(),
        (1..=53).rev().collect::<Vec<_>>()
    );
    assert_eq!(
        options.last().unwrap().url.as_deref(),
        Some("https://school.example.edu/1")
    );
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn regression_distinct_confirmed_targets_survive_new_shared_email(db: SqlitePool) {
    let (site, c) = review_fixture(&db).await;
    for path in ["/one", "/two"] {
        observation(
            &db,
            &c,
            &site,
            path,
            PageKind::List,
            observed("张三", &[], None),
        )
        .await;
        let id = *candidate_ids(&db).await.last().unwrap();
        review::accept(
            &db,
            &ReviewInput {
                ids: vec![id],
                decisions: Decisions {
                    matches: vec![mentor::import::GroupChoice { row: 1, id: None }],
                    ..Default::default()
                },
            },
            2,
        )
        .await
        .unwrap();
    }
    let ids = candidate_ids(&db).await;
    let targets: Vec<_> =
        sqlx::query_scalar::<_, Option<i64>>("SELECT mentor_id FROM candidate ORDER BY id")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_ne!(targets[0], targets[1]);
    for path in ["/one", "/two"] {
        observation(
            &db,
            &c,
            &site,
            path,
            PageKind::Profile,
            observed("张三", &["z@example.edu"], None),
        )
        .await;
    }
    assert_eq!(candidate_ids(&db).await, ids);
    assert_eq!(
        sqlx::query_scalar::<_, Option<i64>>("SELECT mentor_id FROM candidate ORDER BY id")
            .fetch_all(&db)
            .await
            .unwrap(),
        targets
    );
    for id in &ids {
        let input = ReviewInput {
            ids: vec![*id],
            decisions: Decisions::default(),
        };
        let preview = review::preview(&db, &input).await.unwrap();
        assert_eq!(
            preview.errors.len(),
            1,
            "a single candidate must expose both prior targets"
        );
        assert_eq!(review::accept(&db, &input, 3).await.unwrap().accepted, 0);
    }
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn regression_merged_profile_identity_routes_late_results_and_removes_queued_work(
    db: SqlitePool,
) {
    let (site, c) = review_fixture(&db).await;
    observation(
        &db,
        &c,
        &site,
        "/one",
        PageKind::List,
        observed("张三", &[], None),
    )
    .await;
    observation(
        &db,
        &c,
        &site,
        "/two",
        PageKind::List,
        observed("张三", &["z@example.edu"], None),
    )
    .await;
    observation(
        &db,
        &c,
        &site,
        "/one",
        PageKind::Profile,
        observed("张三", &["z@example.edu"], None),
    )
    .await;
    assert_eq!(candidate_ids(&db).await.len(), 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM crawl_page WHERE kind='profile' AND work_state='queued'"
        )
        .fetch_one(&db)
        .await
        .unwrap(),
        0,
        "unstarted duplicate profiles must be removed at merge time"
    );
    observation(
        &db,
        &c,
        &site,
        "/two",
        PageKind::Profile,
        observed("San Zhang", &[], None),
    )
    .await;
    let ids = candidate_ids(&db).await;
    assert_eq!(ids.len(), 1);
    let detail = review::detail(&db, ids[0]).await.unwrap().unwrap();
    assert!(
        detail
            .candidate
            .fields
            .aliases
            .contains(&"San Zhang".into())
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM crawl_page WHERE kind='profile' AND work_state='queued'"
        )
        .fetch_one(&db)
        .await
        .unwrap(),
        0
    );
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn regression_profile_scalars_replace_list_snippets_and_reopen_accepted_review(
    db: SqlitePool,
) {
    let (site, c) = review_fixture(&db).await;
    let fields = |research: &str, title: &str| MentorFields {
        affiliations: vec![mentor::Affiliation {
            title: Some(title.into()),
            ..Default::default()
        }],
        ..observed("张三", &["z@example.edu"], Some(research))
    };
    observation(
        &db,
        &c,
        &site,
        "/one",
        PageKind::List,
        fields("机器学习", "讲师"),
    )
    .await;
    let id = candidate_ids(&db).await[0];
    review::accept(
        &db,
        &ReviewInput {
            ids: vec![id],
            decisions: Decisions::default(),
        },
        2,
    )
    .await
    .unwrap();
    let full = "机器学习、隐私计算与大模型安全，主持可信人工智能项目";
    observation(
        &db,
        &c,
        &site,
        "/one",
        PageKind::Profile,
        fields(full, "教授"),
    )
    .await;
    observation(
        &db,
        &c,
        &site,
        "/one",
        PageKind::List,
        fields("后到的名单片段", "副教授"),
    )
    .await;
    observation(
        &db,
        &c,
        &site,
        "/one",
        PageKind::Profile,
        fields("同级后到的方向", "研究员"),
    )
    .await;
    let detail = review::detail(&db, id).await.unwrap().unwrap().candidate;
    assert_eq!(detail.fields.research.as_deref(), Some(full));
    assert_eq!(detail.fields.affiliations[0].title.as_deref(), Some("教授"));
    assert_eq!(detail.review, "pending");
    let local = mentor::get(&db, detail.mentor_id.unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(local.research.as_deref(), Some("机器学习"));
    assert_eq!(
        review::preview(
            &db,
            &ReviewInput {
                ids: vec![id],
                decisions: Decisions::default()
            }
        )
        .await
        .unwrap()
        .differences
        .len(),
        2
    );
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn regression_invalid_name_is_corrected_or_fails_with_page_reason(db: SqlitePool) {
    let site = Site::new().await;
    site.page("/", "<title>教师名单</title><p>张三 z@example.edu</p>");
    let mut p = Provider::start().await.unwrap();
    let config = config(&db, &p.address).await;
    for correct in [false, true] {
        let id = queue::start(&db, &[site.url("/")], config, 1)
            .await
            .unwrap();
        let c = queue::next(&db).await.unwrap().unwrap();
        let process = tokio::spawn({
            let db = db.clone();
            let c = c.clone();
            let port = site.port;
            async move {
                super::process(
                    &db,
                    &net::Net::new(vec![]).unwrap(),
                    &Pages::local(port),
                    &Turns::default(),
                    &c,
                    &Cooldown::new(Arc::default()),
                )
                .await
                .unwrap()
            }
        });
        let server = tokio::spawn(async move {
            let mut invalid = false;
            while let Some(r) = p.requests.recv().await {
                let system = r.body["messages"][0]["content"].as_str().unwrap();
                let answer = if system.starts_with("你整理学院") {
                    json!({"kind":"list","next":[]}).to_string()
                } else if invalid && correct {
                    json!({
                        "people": [
                            listed("张三", Some(999), vec!["z@example.edu", "invented@example.edu"])
                        ]
                    })
                    .to_string()
                } else {
                    invalid = true;
                    include_str!("../../tests/model/invalid-name.json").into()
                };
                r.respond.send(Response::answer(&answer)).ok();
            }
            p
        });
        let outcome = tokio::time::timeout(GUARD_TIMEOUT, process)
            .await
            .unwrap()
            .unwrap();
        if correct {
            assert!(matches!(outcome, Outcome::Done(ref page) if page.people.len()==1));
        } else {
            assert!(
                matches!(outcome, Outcome::Failed(ref reason) if reason.message.contains("姓名"))
            );
        }
        save(&db, &c, &outcome).await.unwrap();
        finish(&db, now()).await.unwrap();
        let detail = queue::detail(&db, id, 1, false).await.unwrap().unwrap();
        assert_eq!(
            detail.pages[0].status,
            if correct {
                PageStatus::Success
            } else {
                PageStatus::Failed
            }
        );
        server.abort();
        // A fresh fixture isolates the second run from the first server's lifetime.
        if !correct {
            p = Provider::start().await.unwrap();
            sqlx::query("UPDATE model_config SET base_url=? WHERE id=?")
                .bind(&p.address)
                .bind(config)
                .execute(&db)
                .await
                .unwrap();
        } else {
            break;
        }
    }
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn regression_deleted_or_changed_config_requeues_and_requires_current_consent(
    db: SqlitePool,
) {
    let site = Site::new().await;
    site.page("/", "<title>学院</title><p>教师名单</p>");
    let fetcher = Pages::local(site.port);
    for deleted in [true, false] {
        let mut old_provider = Provider::start().await.unwrap();
        let mut new_provider = Provider::start().await.unwrap();
        let old_id = config(&db, &old_provider.address).await;
        let fallback = config(&db, &new_provider.address).await;
        sqlx::query("UPDATE model_config SET agreed=0 WHERE id=?")
            .bind(fallback)
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("UPDATE profile SET default_model_config_id=?")
            .bind(fallback)
            .execute(&db)
            .await
            .unwrap();
        let crawl = queue::start(&db, &[site.url("/")], old_id, 1)
            .await
            .unwrap();
        let c = queue::next(&db).await.unwrap().unwrap();
        let chosen = model::selected(&db, Some(old_id), model::Feature::Crawl)
            .await
            .unwrap()
            .unwrap();
        let turns = Turns::default();
        let permits = [
            turns
                .take(
                    &chosen.access,
                    &chosen.model,
                    model::background::Priority::Background,
                )
                .await,
            turns
                .take(
                    &chosen.access,
                    &chosen.model,
                    model::background::Priority::Background,
                )
                .await,
            turns
                .take(
                    &chosen.access,
                    &chosen.model,
                    model::background::Priority::Background,
                )
                .await,
        ];
        let net = net::Net::new(vec![]).unwrap();
        let cooldown = Cooldown::new(Arc::default());
        let reader = Reader {
            writes: &fetcher.writes,
            db: &db,
            net: &net,
            turns: &turns,
            claim: &c,
            chosen: &chosen,
            cooldown: &cooldown,
            failed: watch::Sender::new(false),
        };
        let ask = reader.ask::<extract::Route>(
            "route",
            extract::ROUTE,
            json!({"title":"学院","links":[]}),
        );
        tokio::pin!(ask);
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(ask.as_mut().poll(cx).is_pending()))
                .await
        );
        if deleted {
            model::delete(&db, old_id).await.unwrap();
        } else {
            let fields = model::ModelForm {
                name: "修改后的模型".into(),
                provider: "custom".into(),
                base_url: new_provider.address.clone(),
                model: "fixture".into(),
                region: None,
                currency: None,
                prices: model::pricing::Prices::default(),
                off_peak: model::pricing::Prices::default(),
            }
            .check()
            .unwrap();
            model::update(&db, old_id, &fields, "fixture")
                .await
                .unwrap();
        }
        drop(permits);
        tokio::select! {
            result = &mut ask => assert!(matches!(result, Err(ReadError::Gone))),
            _ = old_provider.requests.recv() => {
                panic!("started a request using stale configuration")
            }
        }
        save(&db, &c, &Outcome::Gone).await.unwrap();
        assert!(
            !queue::valid(&mut db.acquire().await.unwrap(), &c)
                .await
                .unwrap()
        );
        let fresh = queue::next(&db).await.unwrap().unwrap();
        let chosen = model::selected(&db, fresh.model_id, model::Feature::Crawl)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(chosen.id, if deleted { fallback } else { old_id });
        let outcome = process(&db, &net, &fetcher, &turns, &fresh, &cooldown)
            .await
            .unwrap();
        assert!(matches!(outcome, Outcome::Pause(_, _)));
        save(&db, &fresh, &outcome).await.unwrap();
        assert!(old_provider.requests.try_recv().is_err());
        assert!(new_provider.requests.try_recv().is_err());
        model::agree(
            &db,
            chosen.id,
            &model::Source::check("custom", None, &new_provider.address).unwrap(),
        )
        .await
        .unwrap();
        queue::control(&db, crawl, "resume", 2).await.unwrap();
        let fresh = queue::next(&db).await.unwrap().unwrap();
        let read = process(&db, &net, &fetcher, &turns, &fresh, &cooldown);
        let answer = async {
            new_provider
                .requests
                .recv()
                .await
                .unwrap()
                .respond
                .send(Response::answer(r#"{"kind":"other","next":[]}"#))
                .ok();
        };
        let (outcome, ()) = tokio::join!(read, answer);
        save(&db, &fresh, &outcome.unwrap()).await.unwrap();
        finish(&db, now()).await.unwrap();
    }
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn regression_supplement_preview_names_target_and_added_content(db: SqlitePool) {
    let (site, c) = review_fixture(&db).await;
    let local = MentorFields {
        affiliations: vec![mentor::Affiliation {
            university: Some("示例大学".into()),
            ..Default::default()
        }],
        ..observed("张三", &["z@example.edu"], None)
    };
    let target = mentor::create(&db, &local, 1).await.unwrap();
    let mut fields = observed(
        "张三",
        &["z@example.edu", "extra@example.edu"],
        Some("机器学习"),
    );
    fields.homepages = vec!["https://school.example.edu/one".into()];
    fields.affiliations = vec![
        mentor::Affiliation {
            university: Some("示例大学".into()),
            school: Some("计算机学院".into()),
            title: Some("教授".into()),
            ..Default::default()
        },
        mentor::Affiliation {
            university: Some("兼任大学".into()),
            ..Default::default()
        },
    ];
    observation(&db, &c, &site, "/one", PageKind::Profile, fields).await;
    let preview = review::preview(
        &db,
        &ReviewInput {
            ids: candidate_ids(&db).await,
            decisions: Decisions::default(),
        },
    )
    .await
    .unwrap();
    let value = serde_json::to_value(preview).unwrap();
    assert_eq!(value["additions"][0]["target"]["id"], target);
    assert_eq!(value["additions"][0]["target"]["profile"]["name"], "张三");
    assert_eq!(
        value["additions"][0]["fields"]["emails"],
        json!(["extra@example.edu"])
    );
    assert_eq!(
        value["additions"][0]["fields"]["homepages"],
        json!(["https://school.example.edu/one"])
    );
    assert_eq!(value["additions"][0]["fields"]["research"], "机器学习");
    assert_eq!(
        value["additions"][0]["fields"]["affiliations"][0]["before"]["university"],
        "示例大学"
    );
    let units = &value["additions"][0]["fields"]["affiliations"];
    assert_eq!(units[0]["added"]["school"], "计算机学院");
    assert_eq!(units[0]["added"]["title"], "教授");
    assert_eq!(units[0]["added"]["university"], Value::Null);
    assert_eq!(units[1]["before"], Value::Null);
    assert_eq!(units[1]["added"]["university"], "兼任大学");
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_website_asking_to_slow_down_holds_new_pages_and_keeps_the_page(db: SqlitePool) {
    let site = Site::new().await;
    site.pages
        .lock()
        .unwrap()
        .insert("/list".into(), (429, "<p>请稍后再试</p>".into()));
    site.page("/other", "<title>教师名单</title>");
    let p = Provider::start().await.unwrap();
    let config = config(&db, &p.address).await;
    queue::start(&db, &[site.url("/list"), site.url("/other")], config, 1)
        .await
        .unwrap();
    let cooldown = Cooldown::new(Arc::default());
    // The second is claimed as if the runner had looked before the first was limited.
    let mut outcomes = Vec::new();
    for _ in 0..2 {
        let claim = queue::next(&db).await.unwrap().unwrap();
        let outcome = process(
            &db,
            &net::Net::new(vec![]).unwrap(),
            &Pages::local(site.port),
            &Turns::default(),
            &claim,
            &cooldown,
        )
        .await
        .unwrap();
        outcomes.push(outcome);
    }
    assert!(matches!(
        outcomes[..],
        [Outcome::RateLimit(..), Outcome::Wait]
    ));
    assert_eq!(*site.visits.lock().unwrap(), ["/list"]);
}

/// A site with a list long enough to be read in `parts` parts, claimed with the provider at `p`.
async fn long_list(db: &SqlitePool, p: &Provider, parts: usize) -> (Site, Claim) {
    let site = Site::new().await;
    let block = format!(
        "<p>{}</p>",
        "张三 教授 研究方向：机器学习与数据挖掘。".repeat(200)
    );
    site.page(
        "/",
        &format!("<title>教师名单</title>{}", block.repeat(parts)),
    );
    let config = config(db, &p.address).await;
    queue::start(db, &[site.url("/")], config, 1).await.unwrap();
    let claim = queue::next(db).await.unwrap().unwrap();
    (site, claim)
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_long_list_limited_in_one_part_holds_back_at_once_and_pays_for_the_rest(db: SqlitePool) {
    let mut p = Provider::start().await.unwrap();
    let (site, claim) = long_list(&db, &p, 3).await;
    let chosen = model::selected(&db, claim.model_id, model::Feature::Crawl)
        .await
        .unwrap()
        .unwrap();
    let turns = Turns::default();
    let cooldown = Cooldown::new(Arc::default());
    // A turn taken elsewhere: two parts go out, and the third waits for a turn.
    let elsewhere = turns
        .take(
            &chosen.access,
            &chosen.model,
            model::background::Priority::Background,
        )
        .await;
    let page = tokio::spawn({
        let (db, turns, cooldown, port) = (db.clone(), turns.clone(), cooldown.clone(), site.port);
        async move {
            let net = net::Net::new(vec![]).unwrap();
            process(&db, &net, &Pages::local(port), &turns, &claim, &cooldown)
                .await
                .unwrap()
        }
    });
    let route = p.requests.recv().await.unwrap();
    route
        .respond
        .send(Response::answer(r#"{"kind":"list","next":[]}"#))
        .ok();
    let first = p.requests.recv().await.unwrap();
    let second = p.requests.recv().await.unwrap();
    first
        .respond
        .send(Response::Refused {
            status: 429,
            message: "slow down".into(),
            retry_after: Some(60),
        })
        .ok();
    // The limit holds back as soon as it is met, not once the page is done.
    while cooldown.until().is_none() {
        tokio::task::yield_now().await;
    }
    second
        .respond
        .send(Response::answer(r#"{"people":[]}"#))
        .ok();
    drop(elsewhere);
    tokio::select! {
        outcome = page => assert!(matches!(outcome.unwrap(), Outcome::RateLimit(..))),
        _ = p.requests.recv() => panic!("the part waiting for its turn was sent after the limit"),
    }
    let calls = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM usage")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(calls, 2, "the route and the part that was answered");
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_long_list_limited_in_a_later_part_waits_out_the_limit(db: SqlitePool) {
    let mut p = Provider::start().await.unwrap();
    let (site, claim) = long_list(&db, &p, 2).await;
    let page = tokio::spawn({
        let (db, port) = (db.clone(), site.port);
        async move {
            let net = net::Net::new(vec![]).unwrap();
            let cooldown = Cooldown::new(Arc::default());
            process(
                &db,
                &net,
                &Pages::local(port),
                &Turns::default(),
                &claim,
                &cooldown,
            )
            .await
            .unwrap()
        }
    });
    let route = p.requests.recv().await.unwrap();
    route
        .respond
        .send(Response::answer(r#"{"kind":"list","next":[]}"#))
        .ok();
    let first = p.requests.recv().await.unwrap();
    let second = p.requests.recv().await.unwrap();
    first
        .respond
        .send(Response::Refused {
            status: 500,
            message: "internal error".into(),
            retry_after: None,
        })
        .ok();
    first.completed.await.unwrap();
    second
        .respond
        .send(Response::Refused {
            status: 429,
            message: "slow down".into(),
            retry_after: Some(60),
        })
        .ok();
    // Read again once the limit lifts, not failed for what went wrong in the first part.
    assert!(matches!(page.await.unwrap(), Outcome::RateLimit(..)));
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_call_still_checking_its_claim_when_another_part_fails_is_not_sent(db: SqlitePool) {
    let mut provider = Provider::start().await.unwrap();
    let id = config(&db, &provider.address).await;
    let url = PageUrl::parse("https://school.example.edu/").unwrap();
    queue::start(&db, &[url], id, 1).await.unwrap();
    let claim = queue::next(&db).await.unwrap().unwrap();
    let chosen = model::selected(&db, Some(id), model::Feature::Crawl)
        .await
        .unwrap()
        .unwrap();
    let reader = Reader {
        writes: &db::Writes::default(),
        db: &db,
        net: &net::Net::new(vec![]).unwrap(),
        turns: &Turns::default(),
        claim: &claim,
        chosen: &chosen,
        cooldown: &Cooldown::new(Arc::default()),
        failed: watch::Sender::new(false),
    };
    // With every connection taken, the call has its turn and waits for one to check its claim.
    let mut held = Vec::new();
    for _ in 0..db.options().get_max_connections() {
        held.push(db.acquire().await.unwrap());
    }
    let ask = reader.ask::<extract::Route>("route", extract::ROUTE, json!({}));
    tokio::pin!(ask);
    tokio::select! {
        biased;
        _ = &mut ask => panic!("the call went on without a connection"),
        () = std::future::ready(()) => {}
    }
    reader.failed.send_replace(true);
    drop(held);
    tokio::select! {
        result = ask => assert!(matches!(result, Err(ReadError::Wait))),
        _ = provider.requests.recv() => panic!("the call was sent after another part failed"),
    }
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn regression_retry_after_blocks_inflight_pages_next_request(db: SqlitePool) {
    let site = Site::new().await;
    for path in ["/a", "/b"] {
        site.page(path, "<title>教师名单</title><p>张三 z@example.edu</p>");
    }
    let mut p = Provider::start().await.unwrap();
    let config = config(&db, &p.address).await;
    queue::start(&db, &[site.url("/a"), site.url("/b")], config, 1)
        .await
        .unwrap();
    let cooldown = Cooldown::new(Arc::default());
    let turns = Turns::default();
    let start = |claim: Claim| {
        let (db, cooldown, turns, port) = (db.clone(), cooldown.clone(), turns.clone(), site.port);
        tokio::spawn(async move {
            process(
                &db,
                &net::Net::new(vec![]).unwrap(),
                &Pages::local(port),
                &turns,
                &claim,
                &cooldown,
            )
            .await
            .unwrap()
        })
    };
    let a = start(queue::next(&db).await.unwrap().unwrap());
    let b = start(queue::next(&db).await.unwrap().unwrap());
    let first = p.requests.recv().await.unwrap();
    let second = p.requests.recv().await.unwrap();
    let guard = tokio::spawn(async {
        loop {
            tokio::task::yield_now().await;
        }
    });
    tokio::time::pause();
    first
        .respond
        .send(Response::Refused {
            status: 429,
            message: "slow down".into(),
            retry_after: Some(60),
        })
        .ok();
    // Whichever page received 429 finishes first. The other page remains in flight.
    tokio::pin!(a, b);
    let (a_limited, result) = tokio::select! {
        result = &mut a => (true, result.unwrap()),
        result = &mut b => (false, result.unwrap()),
    };
    let Outcome::RateLimit(limited, _, round) = result else {
        panic!("the first page was limited");
    };
    // As the run loop does with a limit that does not pause the crawl.
    cooldown.defer(1, round, limited.wait);
    second
        .respond
        .send(Response::answer(r#"{"kind":"list","next":[]}"#))
        .ok();
    let chosen = model::selected(&db, Some(config), model::Feature::Crawl)
        .await
        .unwrap()
        .unwrap();
    let permit = cooldown.acquire(&turns, &chosen);
    tokio::pin!(permit);
    assert!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(permit.as_mut().poll(cx).is_pending()))
            .await,
        "new model work must wait after the other page was limited"
    );
    // The in-flight route's usage is saved while further extraction waits.
    loop {
        if sqlx::query_scalar::<_, i64>("SELECT count(*) FROM usage")
            .fetch_one(&db)
            .await
            .unwrap()
            == 1
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    tokio::time::advance(Duration::from_secs(59)).await;
    assert!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(permit.as_mut().poll(cx).is_pending()))
            .await
    );
    assert!(p.requests.try_recv().is_err());
    tokio::time::advance(Duration::from_secs(2)).await;
    drop(permit.await);
    tokio::time::resume();
    guard.abort();
    let next = tokio::time::timeout(GUARD_TIMEOUT, p.requests.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        next.body["messages"][0]["content"]
            .as_str()
            .unwrap()
            .starts_with("你从教师")
    );
    next.respond.send(Response::answer(r#"{"people":[]}"#)).ok();
    let outcome = if a_limited {
        b.await.unwrap()
    } else {
        a.await.unwrap()
    };
    assert!(matches!(outcome, Outcome::Done(ref p) if p.people.is_empty()));
}
#[sqlx::test(migrator = "db::MIGRATOR")]
async fn regression_background_includes_old_running_paused_and_failed_crawls(db: SqlitePool) {
    let (site, c) = review_fixture(&db).await;
    let paused = queue::start(&db, &[site.url("/paused")], c.model_id.unwrap(), 2)
        .await
        .unwrap();
    queue::control(&db, paused, "pause", 3).await.unwrap();
    let failed: i64 = sqlx::query_scalar(
        r#"INSERT INTO crawl (kind, state, error, created_at, finished_at)
        VALUES ('agent', 'failed', json_object('message', '打不开'), 3, 4)
        RETURNING id"#,
    )
    .fetch_one(&db)
    .await
    .unwrap();
    for n in 0..51 {
        sqlx::query(
            "INSERT INTO crawl(kind,state,created_at,finished_at) VALUES('agent','done',?,?)",
        )
        .bind(10 + n)
        .bind(10 + n)
        .execute(&db)
        .await
        .unwrap();
    }
    let today: i64 = sqlx::query_scalar(
        r#"INSERT INTO crawl (kind, state, created_at, finished_at)
        VALUES ('agent', 'done', 150, 150)
        RETURNING id"#,
    )
    .fetch_one(&db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO crawl(kind,state,created_at,finished_at) VALUES('agent','done',200,200)",
    )
    .execute(&db)
    .await
    .unwrap();
    let work = queue::work(&db, 100, 200).await.unwrap();
    assert_eq!(
        work.iter()
            .filter(|c| c.state == queue::CrawlState::Done)
            .map(|c| c.id)
            .collect::<Vec<_>>(),
        vec![today]
    );
    for id in [c.crawl, paused, failed] {
        assert!(work.iter().any(|c| c.id == id), "old work {id} is absent");
    }
    assert!(
        queue::detail(&db, c.crawl, 1, false)
            .await
            .unwrap()
            .is_some()
    );
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn candidate_classification_pages_follow_normalized_names_schools_and_mentor_edits(
    db: SqlitePool,
) {
    let (site, c) = review_fixture(&db).await;
    let affiliation = |university: &str| mentor::Affiliation {
        university: Some(university.into()),
        ..Default::default()
    };
    let mut local = MentorFields {
        name: "张三".into(),
        aliases: vec!["Ａ  B".into()],
        affiliations: vec![affiliation("ＡＢ 大学")],
        ..Default::default()
    };
    let target = mentor::create(&db, &local, 1).await.unwrap();
    for n in 0..55 {
        observation(
            &db,
            &c,
            &site,
            &format!("/person/{n}"),
            PageKind::Profile,
            MentorFields {
                name: "A B".into(),
                affiliations: vec![affiliation("AB   大学")],
                ..Default::default()
            },
        )
        .await;
    }
    let mut query = CandidateQuery {
        selection: CandidateSelection {
            crawl_id: Some(c.crawl),
            processed: false,
            filter: CandidateFilter::Maybe,
            keyword: None,
        },
        page: 1,
    };
    let first = review::list(&db, &query).await.unwrap();
    assert_eq!((first.total, first.candidates.len()), (55, 50));
    query.page = 2;
    let second = review::list(&db, &query).await.unwrap();
    assert_eq!((second.total, second.candidates.len()), (55, 5));
    assert!(
        second
            .candidates
            .iter()
            .all(|c| c.id > first.candidates.last().unwrap().id)
    );
    local.affiliations = vec![affiliation("另一所大学")];
    mentor::update(&db, target, &local, 2).await.unwrap();
    assert_eq!(review::list(&db, &query).await.unwrap().total, 0);
    query.selection.filter = CandidateFilter::New;
    let new = review::list(&db, &query).await.unwrap();
    assert_eq!((new.total, new.candidates.len()), (55, 5));
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn candidate_categories_do_not_depend_on_other_rows_on_the_page(db: SqlitePool) {
    let (site, c) = review_fixture(&db).await;
    mentor::create(&db, &observed("李四", &[], None), 1)
        .await
        .unwrap();
    for (name, path) in [("张三", "/zhang"), ("李四", "/li")] {
        observation(
            &db,
            &c,
            &site,
            path,
            PageKind::Profile,
            observed(name, &["shared@example.edu"], None),
        )
        .await;
    }
    let mut q = CandidateQuery {
        selection: CandidateSelection {
            crawl_id: Some(c.crawl),
            processed: false,
            filter: CandidateFilter::All,
            keyword: None,
        },
        page: 1,
    };
    let all = review::list(&db, &q).await.unwrap();
    assert_eq!(all.candidates[0].category, CandidateFilter::New);
    assert_eq!(all.candidates[1].category, CandidateFilter::Maybe);
    for filter in [CandidateFilter::New, CandidateFilter::Maybe] {
        q.selection.filter = filter;
        let page = review::list(&db, &q).await.unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.candidates[0].category, filter);
    }
}

mod enrichment;

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn restored_key_pauses_crawl_instead_of_failing_pages(db: SqlitePool) {
    let site = Site::new().await;
    site.page("/", "<title>学院</title><p>师资介绍</p>");
    let config = config(&db, "http://127.0.0.1:1/v1").await;
    sqlx::query("UPDATE model_config SET api_key = NULL")
        .execute(&db)
        .await
        .unwrap();
    let id = queue::start(&db, &[site.url("/")], config, 1)
        .await
        .unwrap();
    let claim = queue::next(&db).await.unwrap().unwrap();
    let result = process(
        &db,
        &net::Net::new(vec![]).unwrap(),
        &Pages::local(site.port),
        &Turns::default(),
        &claim,
        &Cooldown::new(Arc::default()),
    )
    .await
    .unwrap();
    save(&db, &claim, &result).await.unwrap();
    let row =
        sqlx::query_as::<_, (String, Option<String>)>("SELECT state,error FROM crawl WHERE id=?")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(row.0, "paused");
    assert!(row.1.unwrap().contains("重新填写"));
}

impl Pages {
    fn local(port: u16) -> Self {
        Self {
            writes: db::Writes::default(),
            http: Fetcher::local(port),
            browser: None,
            recognize,
        }
    }
}

/// Reads what an image made by `image` says.
fn recognize(image: &[u8]) -> Result<String, String> {
    let at = image
        .windows(5)
        .position(|w| w == b"TEXT:")
        .ok_or("no text in the image")?;
    Ok(String::from_utf8_lossy(&image[at + 5..]).into())
}

/// A PNG this size, as far as its header goes, that says `text`.
fn image(width: u32, height: u32, text: &str) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    png.extend(width.to_be_bytes());
    png.extend(height.to_be_bytes());
    png.extend(b"\x08\x02\0\0\0TEXT:");
    png.extend(text.as_bytes());
    png
}

mod browser;
mod embedded;

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn deleting_a_crawl_removes_pages_and_pending_candidates_but_keeps_reviewed_data(
    db: SqlitePool,
) {
    let site = Site::new().await;
    let model = config(&db, "http://127.0.0.1:1").await;
    let crawl = queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let claim = queue::next(&db).await.unwrap().unwrap();
    let material = crate::parse(&site.url("/"), "<p>导师名单</p>");
    save_material(&db, &claim, &material, PageMethod::Http)
        .await
        .unwrap();
    let page = claim.id;
    let mentor = sqlx::query_scalar!(
        r#"INSERT INTO mentor (name, created_at, updated_at)
        VALUES ('已接受的导师', 1, 1)
        RETURNING id"#
    )
    .fetch_one(&db)
    .await
    .unwrap();
    sqlx::query!(
        r#"INSERT INTO candidate (crawl_id, list_page_id, review, enrich, name)
        VALUES (?, ?, 'pending', 'none', '待审核')"#,
        crawl,
        page
    )
    .execute(&db)
    .await
    .unwrap();
    let accepted = sqlx::query_scalar!(
        r#"INSERT INTO candidate (crawl_id, list_page_id, review, mentor_id, enrich, name,
            emails, affiliations, homepages, aliases)
        VALUES (?, ?, 'accepted', ?, 'done', '已接受',
            '["a@pku.edu.cn","b@pku.edu.cn"]',
            '[{"university":"北京大学","title":"教授"}]',
            '["https://cs.pku.edu.cn/a"]', '["Alice"]')
        RETURNING id"#,
        crawl,
        page,
        mentor
    )
    .fetch_one(&db)
    .await
    .unwrap();
    sqlx::query!("DELETE FROM crawl WHERE id = ?", crawl)
        .execute(&db)
        .await
        .unwrap();
    assert!(
        sqlx::query_scalar!("SELECT id FROM crawl_page WHERE id = ?", page)
            .fetch_optional(&db)
            .await
            .unwrap()
            .is_none()
    );
    let candidate = sqlx::query!(
        "SELECT crawl_id, list_page_id, mentor_id FROM candidate WHERE id = ?",
        accepted
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(candidate.crawl_id, None);
    assert_eq!(candidate.list_page_id, None);
    assert_eq!(candidate.mentor_id, Some(mentor));
    assert_eq!(
        sqlx::query_scalar!("SELECT count(*) FROM candidate")
            .fetch_one(&db)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar!("SELECT count(*) FROM mentor WHERE id = ?", mentor)
            .fetch_one(&db)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar!("SELECT count(*) FROM crawl_run")
            .fetch_one(&db)
            .await
            .unwrap(),
        0
    );
}

/// A candidate's units, those their own page gave, and the schools they are matched by.
async fn units(db: &SqlitePool, name: &str) -> (Vec<Affiliation>, Vec<Affiliation>, Vec<String>) {
    let (Json(units), profile, Json(schools)) = sqlx::query_as::<
        _,
        (
            Json<Vec<Affiliation>>,
            Option<Json<MentorFields>>,
            Json<Vec<String>>,
        ),
    >(
        "SELECT affiliations, profile_fields, match_schools FROM candidate WHERE name = ?",
    )
    .bind(name)
    .fetch_one(db)
    .await
    .unwrap();
    let profile = profile.map(|Json(p)| p.affiliations).unwrap_or_default();
    (units, profile, schools)
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_crawl_s_pages_name_its_school_and_its_candidates_follow_the_user_s_change(
    db: SqlitePool,
) {
    let site = Site::new().await;
    site.page(
        "/",
        "<title>示例大学计算机学院 师资队伍</title><p><a href='/zhang'>张三</a> 教授</p>\
        <p>李四 教授 li@school.example.edu</p>",
    );
    site.page(
        "/zhang",
        "<title>张三</title><p>张三 教授 软件工程系 zhang@school.example.edu</p>\
        <p>另一大学数学学院兼职教授</p>",
    );
    let mut provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    let server = tokio::spawn(async move {
        while let Some(r) = provider.requests.recv().await {
            let system = r.body["messages"][0]["content"].as_str().unwrap();
            let answer = if system.starts_with("你整理学院") {
                json!({"kind":"list","next":[],"university":"示例大学","school":"计算机学院"})
            } else if system.starts_with("你从教师") {
                json!({"people":[
                    listed("张三", Some(1), vec![]),
                    listed("李四", None, vec!["li@school.example.edu"])
                ]})
            } else {
                // A teacher's page takes its own school and college for granted.
                let mut answer = targeted("zhang@school.example.edu");
                answer["affiliations"] = json!([
                    {"university":null,"school":null,"department":"软件工程系","title":"教授"},
                    {"university":"另一大学","school":"数学学院","department":null,"title":"兼职教授"}
                ]);
                answer
            };
            r.respond.send(Response::answer(&answer.to_string())).ok();
        }
    });
    let crawl = queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    while let Some(claim) = queue::next(&db).await.unwrap() {
        process_claim(&db, &site, &claim).await;
    }
    let unit = |university: &str, school: &str| Affiliation {
        university: Some(university.into()),
        school: Some(school.into()),
        ..Default::default()
    };
    let at = |university: &str, school: &str, department: Option<&str>, title: &str| Affiliation {
        department: department.map(Into::into),
        title: Some(title.into()),
        ..unit(university, school)
    };
    assert_eq!(
        crawl_unit(&db, crawl).await.unwrap(),
        Some(unit("示例大学", "计算机学院"))
    );
    let elsewhere = at("另一大学", "数学学院", None, "兼职教授");
    let zhang = vec![
        at("示例大学", "计算机学院", Some("软件工程系"), "教授"),
        elsewhere.clone(),
    ];
    assert_eq!(
        units(&db, "张三").await,
        (
            zhang.clone(),
            zhang,
            vec!["示例大学".into(), "另一大学".into()]
        )
    );
    let li = (
        vec![at("示例大学", "计算机学院", None, "教授")],
        vec![],
        vec!["示例大学".into()],
    );
    assert_eq!(units(&db, "李四").await, li);
    // The user rejects one, then writes the school and college as they should be.
    let rejected = sqlx::query_scalar::<_, i64>("SELECT id FROM candidate WHERE name = '李四'")
        .fetch_one(&db)
        .await
        .unwrap();
    review::reject(&db, &[rejected]).await.unwrap();
    let changed = unit("示例理工大学", "软件学院");
    assert_eq!(
        review::set_unit(&db, crawl, &changed).await.unwrap(),
        review::UnitChange::Set
    );
    assert_eq!(crawl_unit(&db, crawl).await.unwrap(), Some(changed));
    let zhang = vec![
        at("示例理工大学", "软件学院", Some("软件工程系"), "教授"),
        elsewhere,
    ];
    assert_eq!(
        units(&db, "张三").await,
        (
            zhang.clone(),
            zhang,
            vec!["示例理工大学".into(), "另一大学".into()]
        )
    );
    assert_eq!(units(&db, "李四").await, li);
    server.abort();
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_page_read_while_the_user_changes_the_school_is_saved_at_the_new_one(db: SqlitePool) {
    let site = Site::new().await;
    site.page(
        "/",
        "<title>示例大学计算机学院 师资队伍</title><p><a href='/zhang'>张三</a> 教授</p>",
    );
    site.page("/zhang", "<title>张三</title><p>张三 教授 软件工程系</p>");
    let mut provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    let changed = Affiliation {
        university: Some("示例理工大学".into()),
        school: Some("软件学院".into()),
        ..Default::default()
    };
    let server = tokio::spawn({
        let (db, changed) = (db.clone(), changed.clone());
        async move {
            while let Some(r) = provider.requests.recv().await {
                let system = r.body["messages"][0]["content"].as_str().unwrap();
                let answer = if system.starts_with("你整理学院") {
                    json!({"kind":"list","next":[],"university":"示例大学","school":"计算机学院"})
                } else if system.starts_with("你从教师") {
                    json!({"people":[listed("张三", Some(1), vec![])]})
                } else {
                    // The user writes the school and college as they should be while the
                    // teacher's page is being read.
                    let crawl = sqlx::query_scalar::<_, i64>("SELECT id FROM crawl")
                        .fetch_one(&db)
                        .await
                        .unwrap();
                    review::set_unit(&db, crawl, &changed).await.unwrap();
                    let mut answer = targeted("zhang@school.example.edu");
                    answer["affiliations"] = json!([
                        {"university":null,"school":null,"department":"软件工程系","title":"教授"}
                    ]);
                    answer
                };
                r.respond.send(Response::answer(&answer.to_string())).ok();
            }
        }
    });
    queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    while let Some(claim) = queue::next(&db).await.unwrap() {
        process_claim(&db, &site, &claim).await;
    }
    let zhang = vec![Affiliation {
        department: Some("软件工程系".into()),
        title: Some("教授".into()),
        ..changed
    }];
    assert_eq!(
        units(&db, "张三").await,
        (zhang.clone(), zhang, vec!["示例理工大学".into()])
    );
    server.abort();
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_crawl_that_finds_no_teacher_fails_and_says_what_to_try(db: SqlitePool) {
    let site = Site::new().await;
    site.page("/", "<title>学院概况</title><p>本院成立于 1990 年。</p>");
    let mut provider = Provider::start().await.unwrap();
    let model = config(&db, &provider.address).await;
    let server = tokio::spawn(async move {
        while let Some(r) = provider.requests.recv().await {
            let answer = json!({"kind":"other","next":[]});
            r.respond.send(Response::answer(&answer.to_string())).ok();
        }
    });
    queue::start(&db, &[site.url("/")], model, 1).await.unwrap();
    let claim = queue::next(&db).await.unwrap().unwrap();
    process_claim(&db, &site, &claim).await;
    assert!(
        finish(&db, now()).await.unwrap()[0]
            .message
            .contains("没有找到导师")
    );
    let crawl = &queue::list(&db, 1).await.unwrap().crawls[0];
    assert_eq!(crawl.state, queue::CrawlState::Failed);
    let reason = &crawl.error.as_ref().unwrap().message;
    assert!(reason.contains("没有在读到的网页上找到导师"), "{reason}");
    server.abort();
}
