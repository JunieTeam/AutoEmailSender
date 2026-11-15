-- The addresses a page gives behind its links, which its text does not show: another version of a
-- teacher's page is a copy of one read only if it links no others (docs/data-model.md#整理).
ALTER TABLE crawl_page ADD COLUMN linked_emails TEXT NOT NULL DEFAULT '[]'
  CHECK (json_valid(linked_emails) AND json_type(linked_emails) = 'array');
