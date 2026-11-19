-- Every call to a model is kept for the list of calls (docs/data-model.md#个人资料和配置): one
-- whose provider did not say what it took has no tokens, one that failed has its reason, and a
-- call for a mentor says whom. The tokens' NOT NULL could not be dropped otherwise, so the table is
-- rebuilt as 0025 rebuilt crawl.

CREATE TABLE usage_new (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  at INTEGER NOT NULL,
  model_config_id INTEGER REFERENCES model_config(id) ON DELETE SET NULL,
  model TEXT NOT NULL,
  purpose TEXT NOT NULL CHECK (purpose IN ('crawl', 'match', 'draft', 'rewrite', 'fact_check',
    'reply_outcome', 'reply_todos')),
  crawl_id INTEGER REFERENCES crawl(id) ON DELETE SET NULL,
  mentor_id INTEGER REFERENCES mentor(id) ON DELETE SET NULL,
  email_id INTEGER REFERENCES email(id) ON DELETE SET NULL,
  input_tokens INTEGER,
  cached_tokens INTEGER,
  output_tokens INTEGER,
  off_peak INTEGER NOT NULL DEFAULT 0 CHECK (off_peak IN (0, 1)),
  error TEXT CHECK (error IS NULL OR json_valid(error)),
  -- The three counts come together, and a failed call took none that is known.
  CHECK ((input_tokens IS NULL) = (cached_tokens IS NULL)
    AND (input_tokens IS NULL) = (output_tokens IS NULL)),
  CHECK (error IS NULL OR input_tokens IS NULL)
) STRICT;
-- The calls so far for an email were for its mentor.
INSERT INTO usage_new (id, at, model_config_id, model, purpose, crawl_id, mentor_id, email_id,
    input_tokens, cached_tokens, output_tokens, off_peak)
  SELECT u.id, u.at, u.model_config_id, u.model, u.purpose, u.crawl_id, e.mentor_id, u.email_id,
    u.input_tokens, u.cached_tokens, u.output_tokens, u.off_peak
  FROM usage u LEFT JOIN email e ON e.id = u.email_id;
-- The new table counts on from where the old one got (docs/development.md#数据库).
DELETE FROM sqlite_sequence WHERE name = 'usage_new';
INSERT INTO sqlite_sequence (name, seq) SELECT 'usage_new', seq FROM sqlite_sequence WHERE name = 'usage';
DROP TABLE usage;
PRAGMA legacy_alter_table = ON;
ALTER TABLE usage_new RENAME TO usage;
PRAGMA legacy_alter_table = OFF;

CREATE INDEX usage_at ON usage (at);
CREATE INDEX usage_crawl ON usage (crawl_id) WHERE crawl_id IS NOT NULL;
-- Deleting a mentor or an email unlinks its calls, which would otherwise read through the table.
CREATE INDEX usage_mentor ON usage (mentor_id) WHERE mentor_id IS NOT NULL;
CREATE INDEX usage_email ON usage (email_id) WHERE email_id IS NOT NULL;
