//! Mentors: listing, editing, merging, the trash, and importing and exporting tables.

use crate::*;

pub const NO_MENTOR: &str = "找不到这位导师，可能已被永久删除";

#[derive(Debug, Deserialize, Serialize, TS)]
pub struct MentorId {
    pub id: i64,
}

#[derive(Debug, Deserialize, TS)]
pub struct MentorIds {
    pub ids: Vec<i64>,
}

#[derive(Debug, Deserialize, TS)]
pub struct MentorEdit {
    pub id: i64,
    /// The fields as they were when editing began.
    pub before: mentor::MentorFields,
    #[serde(flatten)]
    pub form: mentor::MentorForm,
}

#[derive(Debug, Deserialize, TS)]
pub struct MentorAddress {
    pub id: i64,
    pub address: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MentorMailbox {
    pub id: i64,
    pub mailbox_id: Option<i64>,
}

/// Two mentors listed twice: `other` goes into `keep`.
#[derive(Debug, Deserialize, TS)]
pub struct MentorMerge {
    pub keep: i64,
    pub other: i64,
}

/// A merge as its preview showed it.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PreviewedMerge {
    #[serde(flatten)]
    pub pair: MentorMerge,
    pub discarded_drafts: Vec<email::DraftKind>,
}

#[derive(Debug, Deserialize, TS)]
pub struct MentorNote {
    pub id: i64,
    /// Blank clears the note.
    pub note: String,
}

pub async fn list_mentors(
    ctx: &Context,
    mut query: mentor::MentorQuery,
) -> Result<mentor::MentorPage, CallError> {
    query.filter.check().map_err(CallError::invalid)?;
    Ok(mentor::list(&ctx.db, &query, now()).await?)
}

pub async fn list_mentor_ids(
    ctx: &Context,
    mut filter: mentor::filters::MentorFilter,
) -> Result<Vec<i64>, CallError> {
    filter.check().map_err(CallError::invalid)?;
    Ok(mentor::ids(&ctx.db, &filter, now()).await?)
}

pub async fn mentor_filter_options(
    ctx: &Context,
    _: Nothing,
) -> Result<mentor::filters::MentorFilterOptions, CallError> {
    Ok(mentor::filters::options(&ctx.db).await?)
}

#[derive(Debug, Deserialize, TS)]
pub struct FilterValuesQuery {
    pub field: mentor::filters::AffiliationField,
    /// The filters on show; this field's own is left out.
    #[serde(flatten)]
    #[ts(flatten)]
    pub filter: mentor::filters::MentorFilter,
}

pub async fn mentor_filter_values(
    ctx: &Context,
    mut input: FilterValuesQuery,
) -> Result<mentor::filters::FilterValues, CallError> {
    input.filter.check().map_err(CallError::invalid)?;
    Ok(mentor::filters::values(&ctx.db, input.field, &input.filter, now()).await?)
}

pub async fn mentor_filter_tags(
    ctx: &Context,
    mut filter: mentor::filters::MentorFilter,
) -> Result<mentor::filters::FilterTags, CallError> {
    filter.check().map_err(CallError::invalid)?;
    Ok(mentor::filters::tags(&ctx.db, &filter, now()).await?)
}

pub async fn get_mentor(ctx: &Context, input: MentorId) -> Result<mentor::MentorDetail, CallError> {
    mentor::get(&ctx.db, input.id)
        .await?
        .ok_or_else(|| CallError::not_found(NO_MENTOR))
}

pub async fn set_mentor_note(
    ctx: &Context,
    input: MentorNote,
) -> Result<Option<String>, CallError> {
    let note = Some(input.note).filter(|n| !n.trim().is_empty());
    require_applied(
        mentor::set_note(&ctx.db, input.id, note.as_deref()).await?,
        NO_MENTOR,
    )?;
    Ok(note)
}

pub async fn add_mentor(ctx: &Context, form: mentor::MentorForm) -> Result<MentorId, CallError> {
    let fields = form.check().map_err(CallError::invalid)?;
    let id = mentor::create(&ctx.db, &fields, now()).await?;
    look_up(ctx, id).await?;
    Ok(MentorId { id })
}

/// Returns the mentor as saved, so the page shows it at once.
pub async fn update_mentor(
    ctx: &Context,
    input: MentorEdit,
) -> Result<mentor::MentorDetail, CallError> {
    let fields = input.form.check().map_err(CallError::invalid)?;
    match mentor::edit(&ctx.db, input.id, &input.before, &fields, now()).await? {
        mentor::Edited::Saved => {}
        mentor::Edited::Changed => {
            return Err(CallError::invalid(
                "打开编辑之后，这位导师的资料在别处有了变化（比如合并、导入或导师库更新）。\
                关掉后重新打开，再改一次。"
                    .into(),
            ));
        }
        mentor::Edited::Gone => return Err(CallError::not_found(NO_MENTOR)),
    }
    look_up(ctx, input.id).await?;
    get_mentor(ctx, MentorId { id: input.id }).await
}

pub async fn set_mentor_mailbox(
    ctx: &Context,
    input: MentorMailbox,
) -> Result<mentor::MentorDetail, CallError> {
    let applied = mentor::set_mailbox(&ctx.db, input.id, input.mailbox_id)
        .await
        .map_err(|e| {
            if e.as_database_error()
                .is_some_and(|e| e.is_foreign_key_violation())
            {
                CallError::invalid("这个邮箱已经删除，请刷新后重新选择".into())
            } else {
                e.into()
            }
        })?;
    require_applied(applied, NO_MENTOR)?;
    ctx.sender.wake();
    get_mentor(ctx, MentorId { id: input.id }).await
}

pub async fn add_mentor_email(
    ctx: &Context,
    input: MentorAddress,
) -> Result<mentor::MentorDetail, CallError> {
    let address = input.address.trim();
    if !cloud_api::is_email(address) {
        return Err(CallError::invalid(format!("邮箱地址“{address}”格式不正确")));
    }
    let applied = mentor::add_email(&ctx.db, input.id, address, now()).await?;
    require_applied(applied, NO_MENTOR)?;
    look_up(ctx, input.id).await?;
    get_mentor(ctx, MentorId { id: input.id }).await
}

pub async fn preview_merge(
    ctx: &Context,
    input: MentorMerge,
) -> Result<mentor::merge::Preview, CallError> {
    Ok(mentor::merge::preview(&ctx.db, input.keep, input.other).await?)
}

pub async fn merge_mentors(ctx: &Context, input: PreviewedMerge) -> Result<(), CallError> {
    let MentorMerge { keep, other } = input.pair;
    mentor::merge::merge(&ctx.db, keep, other, &input.discarded_drafts, now()).await?;
    ctx.reply_processor.wake();
    ctx.todo_reminders.wake();
    Ok(())
}

/// Looks up the earlier mail at a mentor's new addresses without waiting for the next round.
pub async fn look_up(ctx: &Context, mentor_id: i64) -> Result<(), CallError> {
    if receive::queued(&ctx.db, mentor_id).await? {
        ctx.receiver.wake();
    }
    Ok(())
}

pub async fn list_trash(
    ctx: &Context,
    query: mentor::trash::TrashQuery,
) -> Result<mentor::MentorPage, CallError> {
    Ok(mentor::trash::list(&ctx.db, &query).await?)
}

pub async fn trash_mentors(ctx: &Context, input: MentorIds) -> Result<(), CallError> {
    mentor::trash::trash(&ctx.db, &input.ids, now()).await?;
    ctx.todo_reminders.wake();
    // Their emails with the official server are canceled once withdrawn.
    ctx.cloud_send.wake();
    Ok(())
}

pub async fn restore_mentors(ctx: &Context, input: MentorIds) -> Result<(), CallError> {
    mentor::trash::restore(&ctx.db, &input.ids, now()).await?;
    ctx.reply_processor.wake();
    ctx.todo_reminders.wake();
    Ok(())
}

pub async fn delete_mentors(ctx: &Context, input: MentorIds) -> Result<(), CallError> {
    mentor::trash::delete(&ctx.db, &input.ids)
        .await
        .map_err(|e| match e {
            // Canceled when trashed, they are withdrawn at the next sync.
            mentor::trash::DeleteError::InCloud(count) => CallError::invalid(format!(
                "有 {count} 封邮件已交给云端，正在撤回。撤回后才能永久删除，请稍后再试。"
            )),
            mentor::trash::DeleteError::Db(e) => e.into(),
        })?;
    ctx.todo_reminders.wake();
    Ok(())
}

impl From<mentor::SaveError> for CallError {
    fn from(e: mentor::SaveError) -> Self {
        match e {
            mentor::SaveError::EmailTaken {
                address,
                name,
                trashed,
            } => CallError::invalid(if trashed {
                format!("邮箱“{address}”已用于回收站里的导师{name}")
            } else {
                format!("邮箱“{address}”已用于导师{name}")
            }),
            mentor::SaveError::Db(e) => e.into(),
        }
    }
}

impl From<mentor::merge::MergeError> for CallError {
    fn from(e: mentor::merge::MergeError) -> Self {
        match e {
            mentor::merge::MergeError::Gone => {
                CallError::not_found("有一位导师已被永久删除或移入回收站")
            }
            mentor::merge::MergeError::Busy { name, count } => CallError::invalid(format!(
                "{name}老师还有 {count} 封邮件会发出（待发送、发送中或发送失败）。等它们发出，\
                或改回草稿后再合并。"
            )),
            mentor::merge::MergeError::Changed => {
                CallError::invalid("预览之后两位导师的草稿有了变化，请看过新的预览再合并".into())
            }
            mentor::merge::MergeError::Db(e) => e.into(),
        }
    }
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ImportFile {
    pub file_name: String,
    pub data: String,
    pub setup: mentor::import::Setup,
}

#[derive(Debug, Deserialize, TS)]
pub struct ImportMentors {
    #[serde(flatten)]
    pub file: ImportFile,
    pub decisions: mentor::import::Decisions,
}

pub async fn read_import(input: &ImportFile) -> Result<Vec<sheet::Sheet>, CallError> {
    let bytes = uploaded_bytes(&input.data)?;
    let name = input.file_name.clone();
    tokio::task::spawn_blocking(move || sheet::read(&name, &bytes))
        .await
        .map_err(CallError::internal)?
        .map_err(|e| match &e {
            sheet::ReadError::Damaged(detail) => {
                CallError::invalid(e.to_string()).logging(format!("reading table: {detail}"))
            }
            _ => CallError::invalid(e.to_string()),
        })
}
#[derive(Debug, Deserialize, TS)]
pub struct PreviewImport {
    #[serde(flatten)]
    pub file: ImportFile,
    #[ts(optional)]
    pub decisions: Option<mentor::import::Decisions>,
}
pub async fn preview_import(
    ctx: &Context,
    input: PreviewImport,
) -> Result<mentor::import::Preview, CallError> {
    let sheets = read_import(&input.file).await?;
    Ok(mentor::import::preview_decisions(
        &ctx.db,
        &sheets,
        &input.file.setup,
        &input.decisions.unwrap_or_default(),
    )
    .await?)
}
impl From<mentor::import::Error> for CallError {
    fn from(error: mentor::import::Error) -> Self {
        match error {
            mentor::import::Error::Invalid(reason) => Self::invalid(reason),
            mentor::import::Error::Database(error) => error.into(),
        }
    }
}

pub async fn import_mentors(
    ctx: &Context,
    input: ImportMentors,
) -> Result<mentor::import::Imported, CallError> {
    let sheets = read_import(&input.file).await?;
    let result =
        mentor::import::import(&ctx.db, &sheets, &input.file.setup, &input.decisions, now())
            .await?;
    tracing::info!(
        created = result.created,
        updated = result.updated,
        "imported mentors"
    );
    Ok(result)
}

#[derive(Debug, Clone, Copy, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum TableFormat {
    Xlsx,
    Csv,
}

#[derive(Debug, Deserialize, TS)]
pub struct ExportMentors {
    pub scope: mentor::export::Scope,
    /// As the list is sorted.
    #[serde(default)]
    #[ts(as = "Option<_>", optional)]
    pub sort: mentor::MentorSort,
    pub format: TableFormat,
    pub note: bool,
}

pub async fn export_mentors(ctx: &Context, mut input: ExportMentors) -> Result<Vec<u8>, CallError> {
    if let mentor::export::Scope::Filter(filter) = &mut input.scope {
        filter.check().map_err(CallError::invalid)?;
    }
    let rows = mentor::export::export(&ctx.db, &input.scope, input.sort, input.note, now()).await?;
    tokio::task::spawn_blocking(move || write_table(rows, input.format))
        .await
        .map_err(CallError::internal)?
}

pub fn write_table(rows: Vec<Vec<String>>, format: TableFormat) -> Result<Vec<u8>, CallError> {
    match format {
        TableFormat::Csv => sheet::write_csv(&rows).map_err(CallError::internal),
        TableFormat::Xlsx => {
            for row in rows.iter().skip(1) {
                for (field, value) in rows[0].iter().zip(row) {
                    if value.chars().count() > 32767 {
                        let advice = if field == "备注" {
                            "改导出 CSV，或取消“包含备注”后重试"
                        } else {
                            "改导出 CSV 后重试"
                        };
                        return Err(CallError::invalid(format!(
                            "{}的“{field}”超过 XLSX 每格 32767 个字的上限，请{advice}",
                            row[0]
                        )));
                    }
                }
            }
            let instructions = mentor::export::instructions(&rows[0]);
            sheet::write_xlsx(&[
                sheet::Sheet {
                    name: "导师".into(),
                    rows,
                },
                instructions,
            ])
            .map_err(CallError::internal)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_cells_identify_the_mentor_and_field_and_allow_csv() {
        for field in ["备注", "研究方向"] {
            let rows = vec![
                vec!["姓名".into(), field.into()],
                vec!["王明".into(), "长".repeat(32768)],
            ];
            let error = write_table(rows.clone(), TableFormat::Xlsx).unwrap_err();
            assert!(matches!(error.code, ErrorCode::Invalid));
            assert!(error.message.contains("王明") && error.message.contains(field));
            assert!(error.message.contains("CSV"));
            if field == "备注" {
                assert!(error.message.contains("包含备注"));
            }
            assert!(write_table(rows, TableFormat::Csv).is_ok());
        }
        let rows = vec![
            vec!["姓名".into(), "备注".into()],
            vec!["王明".into(), "长".repeat(32767)],
        ];
        assert!(write_table(rows, TableFormat::Xlsx).is_ok());
    }
}
