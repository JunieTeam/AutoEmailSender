-- The model configuration a feature uses instead of the default; a feature without a row follows
-- the default (docs/data-model.md#个人资料和配置).
CREATE TABLE feature_model (
  feature TEXT PRIMARY KEY CHECK (feature IN ('crawl', 'match', 'writing', 'check', 'reply')),
  model_config_id INTEGER NOT NULL REFERENCES model_config(id) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;
