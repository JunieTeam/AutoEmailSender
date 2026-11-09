#![cfg(test)]

use std::num::NonZeroU32;

use mentor::filters::{AffiliationField, MatchFilter, MentorFilter, Missing};
use mentor::{Affiliation, ContactStatus, MentorFields, MentorQuery};
use sqlx::SqlitePool;

fn affiliation(
    university: Option<&str>,
    school: Option<&str>,
    department: Option<&str>,
    title: Option<&str>,
) -> Affiliation {
    Affiliation {
        university: university.map(Into::into),
        school: school.map(Into::into),
        department: department.map(Into::into),
        title: title.map(Into::into),
    }
}

async fn seed(db: &SqlitePool) -> [i64; 6] {
    let mut ids = Vec::new();
    for (name, affiliations) in [
        (
            "甲",
            vec![
                affiliation(Some("A"), Some("X"), Some("D"), Some("教授")),
                affiliation(Some("B"), Some("Y"), None, None),
                affiliation(Some("B"), Some("X"), None, None),
            ],
        ),
        (
            "乙",
            vec![affiliation(Some("B"), Some("X"), Some("E"), Some("讲师"))],
        ),
        ("丙", vec![]),
        ("丁", vec![affiliation(None, None, Some("D"), None)]),
        ("戊", vec![affiliation(Some("C"), None, None, None)]),
        (
            "回收",
            vec![affiliation(Some("Z"), Some("Z"), Some("Z"), Some("Z"))],
        ),
    ] {
        ids.push(
            mentor::create(
                db,
                &MentorFields {
                    name: name.into(),
                    affiliations,
                    research: Some("机器学习".into()),
                    ..Default::default()
                },
                0,
            )
            .await
            .unwrap(),
        );
    }
    for (i, score) in [(0, 80), (1, 79), (2, 0), (4, 100), (5, 99)] {
        sqlx::query(
            r#"UPDATE mentor
            SET match_score = ?, matched_at = 0, match_model = 'test'
            WHERE id = ?"#,
        )
        .bind(score)
        .bind(ids[i])
        .execute(db)
        .await
        .unwrap();
    }
    // Queued work does not hide the previous result, or turn it into an unanalyzed mentor.
    sqlx::query(
        r#"UPDATE mentor
        SET match_state = 'queued', match_queued_at = 0, match_priority = 0
        WHERE id IN (?, ?)"#,
    )
    .bind(ids[0])
    .bind(ids[3])
    .execute(db)
    .await
    .unwrap();
    for (name, mentors) in [
        ("A 标签", vec![ids[0]]),
        ("B 标签", vec![ids[0], ids[1]]),
        ("C 标签", vec![ids[1]]),
        ("空标签", vec![]),
        ("回收标签", vec![ids[5]]),
    ] {
        mentor::tags::create(
            db,
            &mentor::tags::TagFields {
                name: name.into(),
                color: "#991b1b".into(),
            },
            &mentors,
        )
        .await
        .unwrap();
    }
    mentor::trash::trash(db, &[ids[5]], 1).await.unwrap();
    ids.try_into().unwrap()
}

async fn ids(db: &SqlitePool, filter: MentorFilter) -> Vec<i64> {
    let found = mentor::list(
        db,
        &MentorQuery {
            filter,
            sort: mentor::MentorSort::Match,
            page: NonZeroU32::MIN,
        },
        0,
    )
    .await
    .unwrap();
    assert_eq!(found.total, found.mentors.len() as i64);
    let mut ids: Vec<_> = found.mentors.into_iter().map(|m| m.id).collect();
    ids.sort();
    ids
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn affiliation_filters_use_any_value_and_independent_affiliations(db: SqlitePool) {
    let [a, b, c, d, e, _] = seed(&db).await;
    assert_eq!(ids(&db, MentorFilter::default()).await, [a, b, c, d, e]);
    assert_eq!(
        ids(
            &db,
            MentorFilter {
                universities: vec![Some("A".into()), Some("C".into())],
                ..Default::default()
            }
        )
        .await,
        [a, e]
    );
    assert_eq!(
        ids(
            &db,
            MentorFilter {
                universities: vec![Some("A".into())],
                schools: vec![Some("Y".into())],
                departments: vec![Some("D".into())],
                titles: vec![Some("教授".into())],
                ..Default::default()
            }
        )
        .await,
        [a]
    );
    assert!(
        ids(
            &db,
            MentorFilter {
                universities: vec![Some("C".into())],
                schools: vec![Some("X".into())],
                ..Default::default()
            }
        )
        .await
        .is_empty()
    );
    assert_eq!(
        ids(
            &db,
            MentorFilter {
                universities: vec![Some("B".into()), None],
                ..Default::default()
            }
        )
        .await,
        [a, b, c, d]
    );
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn missing_means_no_affiliation_fills_the_field(db: SqlitePool) {
    let [_, _, c, d, e, _] = seed(&db).await;
    for (filter, expected) in [
        (
            MentorFilter {
                universities: vec![None],
                ..Default::default()
            },
            vec![c, d],
        ),
        (
            MentorFilter {
                schools: vec![None],
                ..Default::default()
            },
            vec![c, d, e],
        ),
        (
            MentorFilter {
                departments: vec![None],
                ..Default::default()
            },
            vec![c, e],
        ),
        (
            MentorFilter {
                titles: vec![None],
                ..Default::default()
            },
            vec![c, d, e],
        ),
    ] {
        assert_eq!(ids(&db, filter).await, expected);
    }
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn tags_scores_and_existing_filters_also_apply_to_export(db: SqlitePool) {
    let [a, b, c, d, e, _] = seed(&db).await;
    assert_eq!(
        ids(
            &db,
            MentorFilter {
                tags: vec![Some(1), Some(3)],
                ..Default::default()
            }
        )
        .await,
        [a, b]
    );
    assert_eq!(
        ids(
            &db,
            MentorFilter {
                tags: vec![None],
                ..Default::default()
            }
        )
        .await,
        [c, d, e]
    );
    assert_eq!(
        ids(
            &db,
            MentorFilter {
                tags: vec![Some(1), None],
                ..Default::default()
            }
        )
        .await,
        [a, c, d, e]
    );
    assert_eq!(
        ids(
            &db,
            MentorFilter {
                matching: Some(MatchFilter::Unanalyzed),
                ..Default::default()
            }
        )
        .await,
        [d]
    );
    test_fixtures::mail::first_email(&db, a, "sent", 0)
        .await
        .unwrap();
    let filter = MentorFilter {
        status: Some(ContactStatus::Contacted),
        keyword: Some("学习".into()),
        matching: Some(MatchFilter::AtLeast { score: 80 }),
        universities: vec![Some("B".into())],
        schools: vec![Some("Y".into())],
        departments: vec![Some("D".into())],
        titles: vec![Some("教授".into())],
        tags: vec![Some(1)],
        ..Default::default()
    };
    assert_eq!(ids(&db, filter.clone()).await, [a]);
    let rows = mentor::export::export(
        &db,
        &mentor::export::Scope::Filter(filter),
        mentor::MentorSort::Match,
        false,
        0,
    )
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1][0], "甲");
    assert_eq!(
        ids(
            &db,
            MentorFilter {
                matching: Some(MatchFilter::AtLeast { score: 80 }),
                ..Default::default()
            }
        )
        .await,
        [a, e]
    );
}

async fn values(
    db: &SqlitePool,
    field: AffiliationField,
    filter: MentorFilter,
) -> (Vec<(String, i64)>, i64) {
    let found = mentor::filters::values(db, field, &filter, 0)
        .await
        .unwrap();
    let values = found
        .values
        .into_iter()
        .map(|v| (v.value, v.count))
        .collect();
    (values, found.missing)
}

fn counted(values: &[(&str, i64)]) -> Vec<(String, i64)> {
    values.iter().map(|&(v, n)| (v.to_owned(), n)).collect()
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn values_count_distinct_active_mentors_and_sort_by_count_then_value(db: SqlitePool) {
    seed(&db).await;
    for (field, expected, missing) in [
        (
            AffiliationField::University,
            vec![("B", 2), ("A", 1), ("C", 1)],
            2,
        ),
        (AffiliationField::School, vec![("X", 2), ("Y", 1)], 3),
        (AffiliationField::Department, vec![("D", 2), ("E", 1)], 2),
        (AffiliationField::Title, vec![("教授", 1), ("讲师", 1)], 3),
    ] {
        assert_eq!(
            values(&db, field, MentorFilter::default()).await,
            (counted(&expected), missing)
        );
    }
    // Tags no mentor outside the trash has are not offered.
    let tags = mentor::filters::tags(&db, &MentorFilter::default(), 0)
        .await
        .unwrap();
    let listed: Vec<_> = tags
        .tags
        .iter()
        .map(|t| (t.id, t.name.as_str(), t.count))
        .collect();
    assert_eq!(
        listed,
        [(2, "B 标签", 2), (1, "A 标签", 1), (3, "C 标签", 1)]
    );
    assert_eq!(tags.untagged, 3);
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn values_are_those_of_the_mentors_the_other_filters_leave(db: SqlitePool) {
    let [a, b, ..] = seed(&db).await;
    let in_c = MentorFilter {
        universities: vec![Some("C".into())],
        ..Default::default()
    };
    // Only 戊 is at C, and has no school; the university menu itself still offers every one.
    assert_eq!(
        values(&db, AffiliationField::School, in_c.clone()).await,
        (Vec::new(), 1)
    );
    assert_eq!(
        values(&db, AffiliationField::University, in_c).await,
        (counted(&[("B", 2), ("A", 1), ("C", 1)]), 2)
    );
    // With no mentor left, no value and no mentor missing one.
    let nowhere = MentorFilter {
        universities: vec![Some("无此校".into())],
        ..Default::default()
    };
    assert_eq!(
        values(&db, AffiliationField::School, nowhere.clone()).await,
        (Vec::new(), 0)
    );
    let tags = mentor::filters::tags(&db, &nowhere, 0).await.unwrap();
    assert!(tags.tags.is_empty());
    assert_eq!(tags.untagged, 0);
    let lecturers = MentorFilter {
        titles: vec![Some("讲师".into())],
        ..Default::default()
    };
    assert_eq!(
        values(&db, AffiliationField::University, lecturers.clone()).await,
        (counted(&[("B", 1)]), 0)
    );
    let tags = mentor::filters::tags(&db, &lecturers, 0).await.unwrap();
    let listed: Vec<_> = tags
        .tags
        .iter()
        .map(|t| (t.name.as_str(), t.count))
        .collect();
    assert_eq!(listed, [("B 标签", 1), ("C 标签", 1)]);
    assert_eq!(tags.untagged, 0);
    // A value's count is what choosing it leaves.
    let chosen = MentorFilter {
        universities: vec![Some("B".into())],
        ..lecturers
    };
    assert_eq!(ids(&db, chosen).await, [b]);
    let keyword = MentorFilter {
        keyword: Some("甲".into()),
        ..Default::default()
    };
    assert_eq!(
        values(&db, AffiliationField::School, keyword.clone()).await,
        (counted(&[("X", 1), ("Y", 1)]), 0)
    );
    assert_eq!(ids(&db, keyword).await, [a]);
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn titles_are_listed_and_matched_by_their_parts(db: SqlitePool) {
    let mut ids_by_title = Vec::new();
    for (name, title) in [
        ("甲", "教授，博士生导师"),
        ("乙", "教授 博导"),
        ("丙", "Associate  Professor"),
        ("丁", "、"),
        // Not beside Chinese, however far from ASCII its letters are.
        ("戊", "Professeur associé HDR"),
        ("己", "副教授 PhD"),
    ] {
        let id = mentor::create(
            &db,
            &MentorFields {
                name: name.into(),
                affiliations: vec![affiliation(Some("A"), None, None, Some(title))],
                ..Default::default()
            },
            0,
        )
        .await
        .unwrap();
        ids_by_title.push(id);
    }
    assert_eq!(
        values(&db, AffiliationField::Title, MentorFilter::default()).await,
        (
            counted(&[
                ("教授", 2),
                ("Associate Professor", 1),
                ("PhD", 1),
                ("Professeur associé HDR", 1),
                ("副教授", 1),
                ("博士生导师", 1),
                ("博导", 1),
            ]),
            1
        )
    );
    let filter = |titles: &[Option<&str>]| MentorFilter {
        titles: titles.iter().map(|t| t.map(Into::into)).collect(),
        ..Default::default()
    };
    assert_eq!(
        ids(&db, filter(&[Some("博士生导师")])).await,
        [ids_by_title[0]]
    );
    assert_eq!(
        ids(&db, filter(&[Some("教授")])).await,
        [ids_by_title[0], ids_by_title[1]]
    );
    // A title of nothing but separators has no parts, so it counts as none.
    assert_eq!(ids(&db, filter(&[None])).await, [ids_by_title[3]]);
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn missing_finds_mentors_without_that_part_of_the_profile(db: SqlitePool) {
    let full = mentor::create(
        &db,
        &MentorFields {
            name: "全".into(),
            emails: vec!["quan@pku.edu.cn".into()],
            homepages: vec!["https://pku.edu.cn/quan".into()],
            research: Some("机器学习".into()),
            ..Default::default()
        },
        0,
    )
    .await
    .unwrap();
    sqlx::query("UPDATE mentor_detail SET papers = '[\"论文\"]' WHERE mentor_id = ?")
        .bind(full)
        .execute(&db)
        .await
        .unwrap();
    let bare = mentor::create(
        &db,
        &MentorFields {
            name: "空".into(),
            ..Default::default()
        },
        0,
    )
    .await
    .unwrap();
    // Papers once found and since removed leave an empty list.
    sqlx::query("UPDATE mentor_detail SET papers = '[]' WHERE mentor_id = ?")
        .bind(bare)
        .execute(&db)
        .await
        .unwrap();
    for missing in [
        Missing::Email,
        Missing::Research,
        Missing::Papers,
        Missing::Homepage,
    ] {
        let filter = MentorFilter {
            missing: Some(missing),
            ..Default::default()
        };
        assert_eq!(ids(&db, filter).await, [bare], "{missing:?}");
    }
}
