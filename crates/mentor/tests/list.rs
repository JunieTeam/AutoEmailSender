// Marks the whole file as test code, so clippy's test allowances cover the helpers too.
#![cfg(test)]

use std::num::NonZeroU32;

use mentor::{
    Affiliation, Contact, ContactStatus, MatchScore, MatchSummary, MentorFields, MentorQuery,
    PAGE_SIZE,
};
use sqlx::SqlitePool;
use test_fixtures::mail::first_email;

fn query(status: Option<ContactStatus>, keyword: Option<&str>, page: u32) -> MentorQuery {
    MentorQuery {
        filter: mentor::filters::MentorFilter {
            status,
            keyword: keyword.map(Into::into),
            ..Default::default()
        },
        sort: mentor::MentorSort::Match,
        page: NonZeroU32::new(page).unwrap(),
    }
}

async fn names(
    db: &SqlitePool,
    status: Option<ContactStatus>,
    keyword: Option<&str>,
) -> Vec<String> {
    let page = mentor::list(db, &query(status, keyword, 1), 0)
        .await
        .unwrap();
    assert_eq!(page.total, page.mentors.len() as i64);
    page.mentors.into_iter().map(|m| m.name).collect()
}

async fn add(db: &SqlitePool, name: &str, university: &str, research: &str) -> i64 {
    let mentor = MentorFields {
        name: name.into(),
        affiliations: vec![at(university, None)],
        research: Some(research.into()),
        ..MentorFields::default()
    };
    mentor::create(db, &mentor, 0).await.unwrap()
}

fn at(university: &str, school: Option<&str>) -> Affiliation {
    Affiliation {
        university: Some(university.into()),
        school: school.map(Into::into),
        ..Affiliation::default()
    }
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn unanswered_filter_uses_latest_sent_time_and_ignores_automatic_replies(db: SqlitePool) {
    let day = 86_400_000;
    let now = 20 * day;
    test_fixtures::mail::mailbox(&mut db.acquire().await.unwrap())
        .await
        .unwrap();
    let mut ids = vec![];
    for name in [
        "超过七天",
        "恰好七天",
        "不到七天",
        "已经回复",
        "已经退信",
        "自动回复",
        "网页发信",
    ] {
        ids.push(add(&db, name, "学校", "研究").await);
    }
    for (i, &id) in ids[..6].iter().enumerate() {
        first_email(
            &db,
            id,
            "sent",
            now - 7 * day
                + match i {
                    0 | 3..=5 => -1,
                    1 => 0,
                    _ => 1,
                },
        )
        .await
        .unwrap();
    }
    sqlx::query("UPDATE email SET state='bounced',error=json_object('message', '地址不存在') WHERE mentor_id=?")
        .bind(ids[4])
        .execute(&db)
        .await
        .unwrap();
    for (i, kind, at) in [
        (3, "reply", now),
        (5, "auto_reply", now),
        (6, "sent", now - 8 * day),
    ] {
        let msg = sqlx::query(
            r#"INSERT INTO message (mailbox_id, folder, uidvalidity, uid, kind,
                message_id, sent_at, from_address, truncated)
            VALUES (1, 'INBOX', 1, ?, ?, ?, ?, 'someone@example.com', 0)"#,
        )
        .bind(i as i64)
        .bind(kind)
        .bind(format!("<filter-{i}@example.com>"))
        .bind(at)
        .execute(&db)
        .await
        .unwrap()
        .last_insert_rowid();
        sqlx::query("INSERT INTO message_mentor(mentor_id,message_id) VALUES(?,?)")
            .bind(ids[i])
            .bind(msg)
            .execute(&db)
            .await
            .unwrap();
    }
    let mut q = query(Some(ContactStatus::Contacted), None, 1);
    q.filter.unanswered_days = NonZeroU32::new(7);
    let found = mentor::list(&db, &q, now).await.unwrap();
    assert_eq!(found.total, 3);
    let actual: std::collections::BTreeSet<_> = found.mentors.iter().map(|m| m.id).collect();
    assert_eq!(actual, [ids[0], ids[5], ids[6]].into_iter().collect());
    // A newer outgoing message restarts the waiting period, including one sent elsewhere.
    first_email(&db, ids[0], "sent", now - day).await.unwrap();
    let found = mentor::list(&db, &q, now).await.unwrap();
    assert_eq!(found.total, 2);
    let rows = mentor::export::export(
        &db,
        &mentor::export::Scope::Filter(mentor::filters::MentorFilter {
            status: q.filter.status,
            unanswered_days: q.filter.unanswered_days,
            ..Default::default()
        }),
        mentor::MentorSort::Match,
        false,
        now,
    )
    .await
    .unwrap();
    assert_eq!(rows.len(), 3);
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn keyword_finds_text_containing_it(db: SqlitePool) {
    add(&db, "张伟", "清华大学", "三维重建与神经渲染").await;
    add(&db, "李静", "北京大学", "自然语言处理、大语言模型对齐").await;
    add(&db, "王磊", "浙江大学", "Machine Learning on Graphs").await;

    assert_eq!(names(&db, None, Some("张伟")).await, ["张伟"]);
    assert_eq!(names(&db, None, Some("重建")).await, ["张伟"]);
    assert_eq!(names(&db, None, Some("语言模型")).await, ["李静"]);
    assert_eq!(names(&db, None, Some("LEARNING")).await, ["王磊"]);
    // Every word must appear, in any field.
    assert_eq!(names(&db, None, Some("清华 神经")).await, ["张伟"]);
    assert!(names(&db, None, Some("清华 语言")).await.is_empty());
    // One character only matches names, otherwise it would hit nearly everyone.
    assert_eq!(names(&db, None, Some("静")).await, ["李静"]);
    assert!(names(&db, None, Some("学")).await.is_empty());
    assert_eq!(names(&db, None, Some("李 语言")).await, ["李静"]);
    assert!(names(&db, None, Some("张 语言")).await.is_empty());
    assert_eq!(names(&db, None, Some("静 李")).await, ["李静"]);
    assert_eq!(names(&db, None, Some("、！")).await.len(), 3);
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn filters_sorts_and_pages(db: SqlitePool) {
    let contacted = add(&db, "甲", "清华大学", "视觉").await;
    let trashed = add(&db, "乙", "清华大学", "视觉").await;
    add(&db, "丙", "清华大学", "视觉").await;
    sqlx::query("UPDATE mentor SET trashed_at = 1 WHERE id = ?")
        .bind(trashed)
        .execute(&db)
        .await
        .unwrap();
    first_email(&db, contacted, "sent", 5).await.unwrap();
    let tag = sqlx::query("INSERT INTO tag (name, color, position) VALUES ('梦导', '#991b1b', 1)")
        .execute(&db)
        .await
        .unwrap()
        .last_insert_rowid();
    sqlx::query("INSERT INTO mentor_tag (mentor_id, tag_id) VALUES (?, ?)")
        .bind(contacted)
        .bind(tag)
        .execute(&db)
        .await
        .unwrap();

    assert_eq!(
        names(&db, Some(ContactStatus::Contacted), None).await,
        ["甲"]
    );
    assert_eq!(names(&db, Some(ContactStatus::None), None).await, ["丙"]);
    let page = mentor::list(&db, &query(Some(ContactStatus::Contacted), None, 1), 0)
        .await
        .unwrap();
    let row = &page.mentors[0];
    assert_eq!(
        row.contact,
        Contact::Contacted {
            opened: None,
            count: 1,
            last_out_at: 5
        }
    );
    assert_eq!(row.tags[0].name, "梦导");

    // Best matches first; mentors not yet analysed come last.
    for i in 0..PAGE_SIZE + 5 {
        let id = add(&db, &format!("导师{i}"), "北京大学", "语言").await;
        sqlx::query(
            "UPDATE mentor SET match_state = NULL, match_score = ?, matched_at = 0, match_model = 'm'
             WHERE id = ?",
        )
        .bind(45 + i)
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    }
    let first = mentor::list(&db, &query(None, None, 1), 0).await.unwrap();
    let second = mentor::list(&db, &query(None, None, 2), 0).await.unwrap();
    assert_eq!(first.total, i64::from(PAGE_SIZE) + 7);
    assert_eq!(
        first.mentors[0].match_state,
        MatchSummary {
            result: Some(MatchScore {
                score: 99,
                basis_changed: false
            }),
            state: None
        }
    );
    assert_eq!(second.mentors.len(), 7);
    assert_eq!(
        second.mentors.last().unwrap().match_state,
        MatchSummary {
            result: None,
            state: None
        }
    );
    let scores: Vec<_> = first
        .mentors
        .iter()
        .chain(&second.mentors)
        .map(|m| &m.match_state)
        .collect();
    assert!(scores.windows(2).all(|w| match (w[0], w[1]) {
        (
            MatchSummary {
                result: Some(MatchScore { score: a, .. }),
                ..
            },
            MatchSummary {
                result: Some(MatchScore { score: b, .. }),
                ..
            },
        ) => a >= b,
        (
            _,
            MatchSummary {
                result: None,
                state: None,
            },
        ) => true,
        _ => false,
    }));
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn keyword_does_not_span_fields(db: SqlitePool) {
    add(&db, "李华", "华东师范大学", "视觉").await;
    add(&db, "李华东", "清华大学", "视觉").await;
    assert_eq!(names(&db, None, Some("李华东")).await, ["李华东"]);
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_mentor_is_found_by_any_name_and_affiliation(db: SqlitePool) {
    let mentor = MentorFields {
        name: "李静".into(),
        aliases: vec!["Jing Li".into()],
        affiliations: vec![
            at("北京大学", Some("计算机学院")),
            at("北京智源人工智能研究院", None),
        ],
        ..MentorFields::default()
    };
    mentor::create(&db, &mentor, 0).await.unwrap();
    add(&db, "张伟", "清华大学", "三维重建").await;

    assert_eq!(names(&db, None, Some("jing")).await, ["李静"]);
    assert_eq!(names(&db, None, Some("智源")).await, ["李静"]);
    assert_eq!(names(&db, None, Some("计算机")).await, ["李静"]);
    let page = mentor::list(&db, &query(None, Some("李静"), 1), 0)
        .await
        .unwrap();
    let row = &page.mentors[0];
    assert_eq!(row.affiliation, Some(at("北京大学", Some("计算机学院"))));
    assert_eq!(row.affiliation_count, 2);
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn the_latest_added_come_first_unless_sorted_by_match(db: SqlitePool) {
    let older = add(&db, "先加的", "清华大学", "视觉").await;
    let newer = add(&db, "后加的", "清华大学", "视觉").await;
    sqlx::query(
        "UPDATE mentor SET match_score = 90, matched_at = 0, match_model = 'm' WHERE id = ?",
    )
    .bind(older)
    .execute(&db)
    .await
    .unwrap();
    for (sort, expected) in [
        (mentor::MentorSort::Added, ["后加的", "先加的"]),
        (mentor::MentorSort::Match, ["先加的", "后加的"]),
    ] {
        let page = mentor::list(
            &db,
            &MentorQuery {
                sort,
                ..query(None, None, 1)
            },
            0,
        )
        .await
        .unwrap();
        let names: Vec<_> = page.mentors.into_iter().map(|m| m.name).collect();
        assert_eq!(names, expected);
        // An exported table keeps the order, whether of the filtered or of the ticked mentors.
        for scope in [
            mentor::export::Scope::Filter(Default::default()),
            mentor::export::Scope::Ids(vec![older, newer]),
        ] {
            let rows = mentor::export::export(&db, &scope, sort, false, 0)
                .await
                .unwrap();
            let exported: Vec<_> = rows[1..].iter().map(|r| r[0].as_str()).collect();
            assert_eq!(exported, expected);
        }
    }
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn addresses_and_notes_are_found_and_follow_their_changes(db: SqlitePool) {
    let id = mentor::create(
        &db,
        &MentorFields {
            name: "张三".into(),
            emails: vec!["zhang.san@pku.edu.cn".into()],
            ..MentorFields::default()
        },
        0,
    )
    .await
    .unwrap();
    add(&db, "李四", "北京大学", "视觉").await;
    for found in [
        "zhang.san@pku.edu.cn",
        "<Zhang.San@PKU.edu.cn>",
        "zhangsan",
        "san@pku",
    ] {
        assert_eq!(names(&db, None, Some(found)).await, ["张三"], "{found}");
    }

    mentor::set_note(&db, id, Some("去年在会议上认识"))
        .await
        .unwrap();
    assert_eq!(names(&db, None, Some("会议")).await, ["张三"]);
    mentor::set_note(&db, id, Some("推荐人是王老师"))
        .await
        .unwrap();
    assert!(names(&db, None, Some("会议")).await.is_empty());
    assert_eq!(names(&db, None, Some("推荐人")).await, ["张三"]);

    mentor::add_email(&db, id, "zs@163.com", 0).await.unwrap();
    assert_eq!(names(&db, None, Some("zs@163.com")).await, ["张三"]);
    // Saving the fields again keeps the note searchable.
    let detail = mentor::get(&db, id).await.unwrap().unwrap();
    let fields = MentorFields {
        name: detail.name,
        emails: detail.emails,
        ..MentorFields::default()
    };
    mentor::update(&db, id, &fields, 0).await.unwrap();
    assert_eq!(names(&db, None, Some("推荐人")).await, ["张三"]);
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn an_emptied_index_is_written_again(db: SqlitePool) {
    add(&db, "张三", "北京大学", "视觉").await;
    let trashed = add(&db, "李四", "北京大学", "视觉").await;
    mentor::trash::trash(&db, &[trashed], 1).await.unwrap();
    sqlx::query("INSERT INTO mentor_search (mentor_search) VALUES ('delete-all')")
        .execute(&db)
        .await
        .unwrap();
    assert!(names(&db, None, Some("北京大学")).await.is_empty());

    assert_eq!(mentor::index_missing(&db).await.unwrap(), 2);
    assert_eq!(names(&db, None, Some("北京大学")).await, ["张三"]);
    assert_eq!(mentor::index_missing(&db).await.unwrap(), 0);
}
