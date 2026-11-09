//! Filters shared by the home list and table export, and the choices shown in the filter menus.

use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use ts_rs::TS;

use crate::{ContactStatus, ReplyOutcomeFilter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MatchFilter {
    AtLeast { score: u8 },
    Unanalyzed,
}

/// Something a mentor's profile lacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(rename_all = "snake_case")]
pub enum Missing {
    Email,
    Research,
    Papers,
    Homepage,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MentorFilter {
    #[ts(optional)]
    pub status: Option<ContactStatus>,
    /// Applied only with status = replied.
    #[ts(optional)]
    pub reply_outcome: Option<ReplyOutcomeFilter>,
    #[ts(optional)]
    pub keyword: Option<String>,
    #[ts(optional)]
    pub unanswered_days: Option<NonZeroU32>,
    /// Applied only with status = contacted: whether the latest letter, tracked, was opened.
    #[ts(optional)]
    pub opened: Option<bool>,
    /// All scores when absent; an unfinished analysis can still have a previous result.
    #[ts(optional)]
    pub matching: Option<MatchFilter>,
    /// Empty lists mean no restriction; None selects mentors with no value in any affiliation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub universities: Vec<Option<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schools: Vec<Option<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub departments: Vec<Option<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub titles: Vec<Option<String>>,
    /// None selects mentors with no tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Option<i64>>,
    /// Mentors without this.
    #[ts(optional)]
    pub missing: Option<Missing>,
}

impl MentorFilter {
    /// Validates and trims user input at the API boundary.
    pub fn check(&mut self) -> Result<(), String> {
        if let Some(MatchFilter::AtLeast { score }) = self.matching
            && !(1..=100).contains(&score)
        {
            return Err("匹配度分数须为 1–100".into());
        }
        for values in [
            &mut self.universities,
            &mut self.schools,
            &mut self.departments,
            &mut self.titles,
        ] {
            for value in values.iter_mut().flatten() {
                *value = value.trim().to_owned();
                if value.is_empty() {
                    return Err("筛选值不能为空，请选择“未填写”".into());
                }
            }
        }
        Ok(())
    }

    pub(crate) fn unanswered_before(&self, now: i64) -> Option<i64> {
        self.unanswered_days
            .map(|days| now - i64::from(days.get()) * 86_400_000)
    }

    pub(crate) fn match_score(&self) -> (Option<u8>, bool) {
        match self.matching {
            Some(MatchFilter::AtLeast { score }) => (Some(score), false),
            Some(MatchFilter::Unanalyzed) => (None, true),
            None => (None, false),
        }
    }
}

#[derive(Debug, Serialize, TS)]
pub struct FilterValue {
    pub value: String,
    pub count: i64,
}

#[derive(Debug, Default, Serialize, TS)]
pub struct FilterValues {
    pub values: Vec<FilterValue>,
    pub missing: i64,
}

#[derive(Debug, Serialize, TS)]
pub struct FilterTag {
    pub id: i64,
    pub name: String,
    pub count: i64,
}

#[derive(Debug, Serialize, TS)]
pub struct FilterTags {
    pub tags: Vec<FilterTag>,
    pub untagged: i64,
}

/// A menu of the values affiliations have.
#[derive(Debug, Clone, Copy, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum AffiliationField {
    University,
    School,
    Department,
    /// Titles in their parts.
    Title,
}

struct Counted {
    value: Option<String>,
    count: i64,
}

struct Tagged {
    id: Option<i64>,
    name: Option<String>,
    count: i64,
}

/// The values of one field among the mentors the other filters leave, with how many of them have
/// each and how many have none: what choosing a value would leave (docs/ui.md#交互).
pub async fn values(
    db: &SqlitePool,
    field: AffiliationField,
    filter: &MentorFilter,
    now: i64,
) -> Result<FilterValues, sqlx::Error> {
    let mut others = filter.clone();
    match field {
        AffiliationField::University => others.universities.clear(),
        AffiliationField::School => others.schools.clear(),
        AffiliationField::Department => others.departments.clear(),
        AffiliationField::Title => others.titles.clear(),
    }
    // Mentors with no value join to NULL, which so counts them. Each field is its own literal
    // query, as SQLx checks only literals.
    let rows = match field {
        AffiliationField::University => crate::filtered_query!(
            db,
            &others,
            now,
            query_as(Counted),
            r#"SELECT v.university AS "value?", count(DISTINCT m.id) AS "count!: i64"
            FROM mentor m
            LEFT JOIN mentor_affiliation v ON v.mentor_id = m.id AND v.university IS NOT NULL"#,
            "GROUP BY v.university ORDER BY 2 DESC, 1",
            fetch_all,
        )?,
        AffiliationField::School => crate::filtered_query!(
            db,
            &others,
            now,
            query_as(Counted),
            r#"SELECT v.school AS "value?", count(DISTINCT m.id) AS "count!: i64"
            FROM mentor m
            LEFT JOIN mentor_affiliation v ON v.mentor_id = m.id AND v.school IS NOT NULL"#,
            "GROUP BY v.school ORDER BY 2 DESC, 1",
            fetch_all,
        )?,
        AffiliationField::Department => crate::filtered_query!(
            db,
            &others,
            now,
            query_as(Counted),
            r#"SELECT v.department AS "value?", count(DISTINCT m.id) AS "count!: i64"
            FROM mentor m
            LEFT JOIN mentor_affiliation v ON v.mentor_id = m.id AND v.department IS NOT NULL"#,
            "GROUP BY v.department ORDER BY 2 DESC, 1",
            fetch_all,
        )?,
        AffiliationField::Title => crate::filtered_query!(
            db,
            &others,
            now,
            query_as(Counted),
            r#"SELECT v.title AS "value?", count(*) AS "count!: i64"
            FROM mentor m
            LEFT JOIN mentor_title v ON v.mentor_id = m.id"#,
            "GROUP BY v.title ORDER BY 2 DESC, 1",
            fetch_all,
        )?,
    };
    let mut values = FilterValues::default();
    for row in rows {
        match row.value {
            Some(value) => values.values.push(FilterValue {
                value,
                count: row.count,
            }),
            None => values.missing = row.count,
        }
    }
    Ok(values)
}

/// The tags among the mentors the other filters leave, as [`values`] does for affiliations.
pub async fn tags(
    db: &SqlitePool,
    filter: &MentorFilter,
    now: i64,
) -> Result<FilterTags, sqlx::Error> {
    let others = MentorFilter {
        tags: Vec::new(),
        ..filter.clone()
    };
    // Untagged mentors join to NULL, which so counts them.
    let rows = crate::filtered_query!(
        db,
        &others,
        now,
        query_as(Tagged),
        r#"SELECT t.id AS "id?", t.name AS "name?", count(*) AS "count!: i64"
        FROM mentor m
        LEFT JOIN mentor_tag mt ON mt.mentor_id = m.id
        LEFT JOIN tag t ON t.id = mt.tag_id"#,
        "GROUP BY t.id ORDER BY count(*) DESC, t.name",
        fetch_all,
    )?;
    let mut result = FilterTags {
        tags: Vec::new(),
        untagged: 0,
    };
    for row in rows {
        match row.id.zip(row.name) {
            Some((id, name)) => result.tags.push(FilterTag {
                id,
                name,
                count: row.count,
            }),
            None => result.untagged = row.count,
        }
    }
    Ok(result)
}

#[derive(Debug, Serialize, TS)]
pub struct MentorFilterOptions {
    /// Whether a letter has gone out tracked, without which filtering by opens finds no one.
    pub tracked: bool,
}

pub async fn options(db: &SqlitePool) -> Result<MentorFilterOptions, sqlx::Error> {
    let tracked = sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM email WHERE tracking_state = 'tracked') AS "tracked!: bool""#
    )
    .fetch_one(db)
    .await?;
    Ok(MentorFilterOptions { tracked })
}
