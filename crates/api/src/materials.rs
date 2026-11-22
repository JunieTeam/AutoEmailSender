//! Materials the student attaches.

use crate::*;

#[derive(Debug, Deserialize, Serialize, TS)]
pub struct MaterialId {
    pub id: i64,
}

/// A file the user picked, as base64: JSON carries no bytes.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UploadedFile {
    pub file_name: String,
    pub data: String,
}

#[derive(Debug, Deserialize, TS)]
pub struct MaterialPurpose {
    pub id: i64,
    pub purpose: material::Purpose,
    pub on: bool,
}

pub const NO_MATERIAL: &str = "找不到这份材料，可能已被删除";

impl From<material::Error> for CallError {
    fn from(e: material::Error) -> Self {
        match e {
            material::Error::Type => CallError::invalid("只能添加 PDF、DOCX 或 DOC 文件".into()),
            material::Error::TooLarge => CallError::invalid("文件超过 20 MB，无法添加".into()),
            material::Error::Unreadable(detail) => CallError::invalid(
                "未能读取文件：文件可能已损坏、设有密码，或格式与扩展名不符".into(),
            )
            .logging(format!("reading a material: {detail}")),
            material::Error::InUse => CallError::invalid(
                "这份材料是待发邮件的附件，邮件发出或改回草稿去掉附件后才能删除".into(),
            ),
            material::Error::TaskInUse { name } => CallError::invalid(format!(
                "这份材料正用于任务“{name}”的草稿生成，请等待生成完成，或停止该任务后再删除"
            )),
            material::Error::Db(e) => e.into(),
            material::Error::File(e) => CallError::internal(format!("material file: {e}")),
        }
    }
}

pub async fn list_materials(
    ctx: &Context,
    _: Nothing,
) -> Result<Vec<material::Material>, CallError> {
    Ok(material::list(&ctx.db).await?)
}

pub async fn add_material(ctx: &Context, input: UploadedFile) -> Result<MaterialId, CallError> {
    let data = uploaded_bytes(&input.data)?;
    let extracted = material::extract(input.file_name.trim(), data).await?;
    let id = material::add(&ctx.db, &ctx.materials, extracted, now()).await?;
    Ok(MaterialId { id })
}

pub async fn set_material_purpose(ctx: &Context, input: MaterialPurpose) -> Result<(), CallError> {
    let applied = material::set_purpose(&ctx.db, input.id, input.purpose, input.on, now()).await?;
    require_applied(applied, NO_MATERIAL)
}

pub async fn get_material_text(
    ctx: &Context,
    input: MaterialId,
) -> Result<Option<String>, CallError> {
    material::text(&ctx.db, input.id)
        .await?
        .ok_or_else(|| CallError::not_found(NO_MATERIAL))
}

pub async fn delete_material(ctx: &Context, input: MaterialId) -> Result<(), CallError> {
    let applied = material::delete(&ctx.db, &ctx.materials, input.id, now()).await?;
    require_applied(applied, NO_MATERIAL)
}

pub fn uploaded_bytes(data: &str) -> Result<Vec<u8>, CallError> {
    if data.len() / 4 * 3 > material::MAX_SIZE + 2 {
        return Err(CallError::invalid(
            "文件超过 20 MB，请缩小文件后重试".into(),
        ));
    }
    let bytes = BASE64_STANDARD.decode(data).map_err(CallError::bad_input)?;
    if bytes.len() > material::MAX_SIZE {
        return Err(CallError::invalid(
            "文件超过 20 MB，请缩小文件后重试".into(),
        ));
    }
    Ok(bytes)
}
