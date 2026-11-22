//! Model configurations and agreeing to what they are sent.

use crate::*;

#[derive(Debug, Deserialize, Serialize, TS)]
pub struct ModelConfigId {
    pub id: i64,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NewModelConfig {
    #[serde(flatten)]
    pub form: model::ModelForm,
    pub api_key: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfigEdit {
    pub id: i64,
    #[serde(flatten)]
    pub form: model::ModelForm,
    /// Blank keeps the saved one, unless the provider or address changed.
    pub api_key: String,
}

/// A provider as filled in, perhaps not saved yet, to list its models.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ModelSource {
    /// The saved configuration being edited, whose API key a blank one stands for while the
    /// provider and address stay the same.
    #[ts(optional)]
    pub id: Option<i64>,
    pub provider: String,
    /// Only read for a provider with regions, whose first it defaults to.
    pub region: Option<String>,
    /// Only read for a custom provider.
    pub base_url: String,
    pub api_key: String,
}

/// A configuration as filled in, perhaps not saved yet.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ModelTest {
    /// The saved configuration being edited, whose API key a blank one stands for while the
    /// provider and address stay the same.
    #[ts(optional)]
    pub id: Option<i64>,
    #[serde(flatten)]
    pub form: model::ModelForm,
    pub api_key: String,
}

pub const NO_MODEL_CONFIG: &str = "找不到这个模型配置，可能已被删除";
pub const NO_API_KEY: &str = "请填写 API Key";

pub async fn list_model_providers(
    _: &Context,
    _: Nothing,
) -> Result<&'static [model::providers::Provider], CallError> {
    Ok(model::providers::PROVIDERS)
}

pub async fn list_model_configs(
    ctx: &Context,
    _: Nothing,
) -> Result<Vec<model::ModelConfig>, CallError> {
    Ok(model::list(&ctx.db, now()).await?)
}

pub async fn add_model_config(
    ctx: &Context,
    input: NewModelConfig,
) -> Result<ModelConfigId, CallError> {
    let fields = input.form.check().map_err(CallError::invalid)?;
    let api_key = entered_or_saved_key(ctx, &input.api_key, None, &fields.source).await?;
    let id = model::create(&ctx.db, &fields, &api_key, now()).await?;
    Ok(ModelConfigId { id })
}

pub async fn update_model_config(ctx: &Context, input: ModelConfigEdit) -> Result<(), CallError> {
    let fields = input.form.check().map_err(CallError::invalid)?;
    let api_key = entered_or_saved_key(ctx, &input.api_key, Some(input.id), &fields.source).await?;
    let applied = model::update(&ctx.db, input.id, &fields, &api_key).await?;
    require_applied(applied, NO_MODEL_CONFIG)
}

pub async fn delete_model_config(ctx: &Context, input: ModelConfigId) -> Result<(), CallError> {
    Ok(model::delete(&ctx.db, input.id).await?)
}

pub async fn set_default_model(ctx: &Context, input: ModelConfigId) -> Result<(), CallError> {
    let applied = profile::set_default_model(&ctx.db, input.id).await?;
    require_applied(applied, "找不到这个模型，可能已被删除")
}

#[derive(Debug, Deserialize, TS)]
pub struct FeatureModel {
    pub feature: model::Feature,
    /// None: the default.
    pub id: Option<i64>,
}

pub async fn set_feature_model(ctx: &Context, input: FeatureModel) -> Result<(), CallError> {
    let applied = model::set_feature(&ctx.db, input.feature, input.id).await?;
    require_applied(applied, "找不到这个模型，可能已被删除")
}

/// What the user was told AI features send to, and agreed to.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ModelAgreement {
    pub id: i64,
    pub provider: String,
    pub region: Option<String>,
    pub base_url: Option<String>,
}

impl ModelAgreement {
    /// Where this is agreed to send, read as a configuration's source is: the confirmation names
    /// the same place the agreement holds for.
    pub(crate) fn source(&self) -> Result<model::Source, CallError> {
        model::Source::check(
            &self.provider,
            self.region.as_deref(),
            self.base_url.as_deref().unwrap_or_default(),
        )
        .map_err(CallError::invalid)
    }
}

pub async fn agree_model(ctx: &Context, input: ModelAgreement) -> Result<(), CallError> {
    let applied = model::agree(&ctx.db, input.id, &input.source()?).await?;
    require_applied(
        applied,
        "这个模型配置刚被修改或删除。请查看新的说明后再确认。",
    )
}

pub async fn list_provider_models(
    ctx: &Context,
    input: ModelSource,
) -> Result<Vec<model::ListedModel>, CallError> {
    let source = model::Source::check(&input.provider, input.region.as_deref(), &input.base_url)
        .map_err(CallError::invalid)?;
    let api_key = entered_or_saved_key(ctx, &input.api_key, input.id, &source).await?;
    model::list_models(&ctx.net, &source.access(Some(api_key)))
        .await
        .map_err(|e| {
            CallError::from(e).logging(format!("model config {:?} of {}", input.id, input.provider))
        })
}

pub async fn test_model(ctx: &Context, input: ModelTest) -> Result<(), CallError> {
    let provider = input.form.provider.clone();
    let fields = input.form.check().map_err(CallError::invalid)?;
    let api_key = entered_or_saved_key(ctx, &input.api_key, input.id, &fields.source).await?;
    let turn = ctx
        .turns
        .take(
            &fields.source.access(Some(api_key)),
            &fields.model,
            model::background::Priority::Interactive,
        )
        .await;
    model::test(&ctx.net, &turn).await.map_err(|e| {
        CallError::from(e).logging(format!(
            "model config {:?} of {provider}: {}",
            input.id, fields.model
        ))
    })
}

/// The API key typed in, else the saved one of the configuration being edited if it is still for
/// the same place. Models on this computer, such as Ollama's, need none.
pub async fn entered_or_saved_key(
    ctx: &Context,
    api_key: &str,
    id: Option<i64>,
    source: &model::Source,
) -> Result<String, CallError> {
    if let Some(api_key) = model::checked_key(api_key).map_err(|e| CallError::invalid(e.into()))? {
        return Ok(api_key);
    }
    let saved = match id {
        Some(id) => model::api_key(&ctx.db, id, source).await?,
        None => None,
    };
    match (saved, source) {
        (Some(Some(api_key)), _) => Ok(api_key),
        (Some(None), _) => Err(CallError::invalid(
            model::ModelError::MissingKey.failure().message,
        )),
        (None, model::Source::Custom(_)) => Ok(String::new()),
        (None, model::Source::Known(..)) => Err(CallError::invalid(NO_API_KEY.into())),
    }
}

impl From<model::ModelError> for CallError {
    fn from(e: model::ModelError) -> Self {
        CallError::external(e.failure(), diagnosed(&e, e.diagnostic()))
    }
}
