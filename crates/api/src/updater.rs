//! Where desktop updates come from, and the update state returned by the application commands.

use failure::Failure;
use serde::Serialize;
use ts_rs::TS;

/// The manifest of the latest GitHub Release, checked first.
pub const GITHUB_MANIFEST: &str =
    "https://github.com/JunieTeam/AutoEmailSender/releases/latest/download/latest.json";

/// The update mirror, for when GitHub cannot be reached or is too slow: it serves `latest.json`,
/// and each version's files under `<version>/`.
pub const MIRROR: &str = "https://dl.auto-email-sender.cn/";

/// The changelog of this version and those before it, newest first.
pub const CHANGELOG: &str = include_str!("../../../CHANGELOG.md");

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    /// Unix milliseconds, or null when the release has no publication date.
    pub published_at: Option<i64>,
    /// The changes since the running version, newest first.
    pub releases: Vec<Release>,
}

impl UpdateInfo {
    /// A manifest's notes are the changelog up to its version (docs/development.md#发布).
    pub fn new(version: String, published_at: Option<i64>, notes: &str, current: &str) -> Self {
        // A version that does not parse is not after any: None is less than Some.
        let current = semver::Version::parse(current).ok();
        let target = semver::Version::parse(&version).ok();
        let releases = releases(notes)
            .into_iter()
            .filter(|r| {
                let release = semver::Version::parse(&r.version).ok();
                release > current && release <= target
            })
            .collect();
        Self {
            version,
            published_at,
            releases,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub version: String,
    /// `YYYY-MM-DD`; none while the version is being prepared.
    pub date: Option<String>,
    pub changes: Vec<Change>,
}

#[derive(Clone, Debug, PartialEq, Serialize, TS)]
pub struct Change {
    pub kind: ChangeKind,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ChangeKind {
    Feature,
    Improvement,
    Fix,
}

/// The releases in a changelog written as CHANGELOG.md is: `## <version>（<date>）`, then
/// `### 新功能`, `### 改进` or `### 修复`, each followed by `- <change>` lines. Other lines are left
/// out, as a manifest's notes come from outside.
pub fn releases(changelog: &str) -> Vec<Release> {
    let mut releases: Vec<Release> = Vec::new();
    let mut kind = None;
    for line in changelog.lines().map(str::trim) {
        if let Some(heading) = line.strip_prefix("## ") {
            let (version, date) = heading
                .split_once('（')
                .map_or((heading, None), |(v, d)| (v, d.strip_suffix('）')));
            releases.push(Release {
                version: version.trim().to_owned(),
                date: date.map(str::to_owned),
                changes: Vec::new(),
            });
            kind = None;
        } else if let Some(heading) = line.strip_prefix("### ") {
            kind = match heading {
                "新功能" => Some(ChangeKind::Feature),
                "改进" => Some(ChangeKind::Improvement),
                "修复" => Some(ChangeKind::Fix),
                _ => None,
            };
        } else if let (Some(text), Some(kind), Some(release)) =
            (line.strip_prefix("- "), kind, releases.last_mut())
        {
            release.changes.push(Change {
                kind,
                text: text.trim().to_owned(),
            });
        }
    }
    releases
}

/// Why a changelog cannot go out with a release, if it cannot: each version is older than the one
/// above it and dated, but for the first while it is being prepared, and lists at least one change,
/// under the headings `releases` reads; anything else would be left out of the app.
pub fn check(changelog: &str) -> Result<(), String> {
    for line in changelog.lines().map(str::trim) {
        let read = line.is_empty()
            || line == "# 更新日志"
            || line.starts_with("## ")
            || line.starts_with("- ")
            || ["### 新功能", "### 改进", "### 修复"].contains(&line);
        if !read {
            return Err(format!("读不懂这一行：{line}"));
        }
    }
    let releases = releases(changelog);
    let listed = changelog.lines().filter(|l| l.trim().starts_with("- ")).count();
    if releases.iter().map(|r| r.changes.len()).sum::<usize>() != listed {
        return Err("有的条目不在“### 新功能”“### 改进”“### 修复”下面".into());
    }
    let mut newer: Option<semver::Version> = None;
    for (i, release) in releases.iter().enumerate() {
        let version = semver::Version::parse(&release.version)
            .map_err(|_| format!("版本号写错了：{}", release.version))?;
        if newer.as_ref().is_some_and(|newer| *newer <= version) {
            return Err(format!("{version} 要写在比它新的版本下面"));
        }
        match &release.date {
            Some(date) if chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok() => {}
            None if i == 0 => {}
            _ => return Err(format!("{version} 的日期要写成（YYYY-MM-DD）")),
        }
        if release.changes.is_empty() {
            return Err(format!("{version} 下面没有写变化"));
        }
        newer = Some(version);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_update_lists_the_changes_of_each_version_after_the_running_one() {
        let notes = "# 更新日志\n\n## 3.1.0（2027-01-02）\n\n### 新功能\n\n- 甲\n\n### 修复\n\n\
            - 乙\n\n## 3.0.1（2026-12-20）\n\n### 改进\n\n- 丙\n\n### 文档\n\n- 不列\n\n\
            ## 3.0.0（2026-12-01）\n\n### 新功能\n\n- 丁\n";
        let info = UpdateInfo::new("3.1.0".into(), None, notes, "3.0.0");
        // A changelog lists nothing past the version it came with.
        assert_eq!(
            UpdateInfo::new("3.0.1".into(), None, notes, "3.0.0").releases[0].version,
            "3.0.1"
        );
        let change = |kind, text: &str| Change {
            kind,
            text: text.into(),
        };
        assert_eq!(
            info.releases,
            [
                Release {
                    version: "3.1.0".into(),
                    date: Some("2027-01-02".into()),
                    changes: vec![
                        change(ChangeKind::Feature, "甲"),
                        change(ChangeKind::Fix, "乙")
                    ],
                },
                Release {
                    version: "3.0.1".into(),
                    date: Some("2026-12-20".into()),
                    changes: vec![change(ChangeKind::Improvement, "丙")],
                },
            ]
        );
        assert!(
            UpdateInfo::new("3.1.0".into(), None, "", "3.0.0")
                .releases
                .is_empty()
        );
    }

    #[test]
    fn a_changelog_goes_out_only_when_the_app_reads_all_of_it() {
        check(CHANGELOG).unwrap();
        let dated = "## 3.0.1（2026-12-20）\n\n### 修复\n\n- 甲\n\n\
            ## 3.0.0（2026-12-01）\n\n### 新功能\n\n- 乙\n";
        check(dated).unwrap();
        check(&format!("## 3.0.2\n\n### 改进\n\n- 丙\n\n{dated}")).unwrap();
        for wrong in [
            dated.replace("2026-12-20", "2026-99-40"),
            dated.replace("（2026-12-01）", ""),
            dated.replace("### 修复", "### 修正"),
            dated.replace("\n- 甲\n", "\n"),
            dated.replace("3.0.1", "2.9.0"),
            dated.replace("### 修复\n\n", ""),
            format!("说明\n\n{dated}"),
        ] {
            assert!(check(&wrong).is_err(), "{wrong}");
        }
    }
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UpdateStatus {
    Current,
    Available {
        update: UpdateInfo,
        error: Option<Failure>,
    },
    Downloading {
        update: UpdateInfo,
        downloaded: u64,
        total: Option<u64>,
    },
    Ready {
        update: UpdateInfo,
    },
}
