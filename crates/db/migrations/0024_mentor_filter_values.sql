-- The parts of titles, which the title filter lists and matches (docs/data-model.md#导师). They are
-- written with the keyword index, so the index is emptied for `mentor::index_missing` to write both.
CREATE TABLE mentor_title (
  mentor_id INTEGER NOT NULL REFERENCES mentor(id) ON DELETE CASCADE,
  title TEXT NOT NULL CHECK (title <> ''),
  PRIMARY KEY (mentor_id, title)
) STRICT, WITHOUT ROWID;
CREATE INDEX mentor_title_value ON mentor_title (title);
DROP INDEX mentor_affiliation_title;
INSERT INTO mentor_search (mentor_search) VALUES ('delete-all');

-- Filtering by what is missing reads these instead of every mentor's long text.
CREATE INDEX mentor_detail_without_research ON mentor_detail (mentor_id) WHERE research IS NULL;
CREATE INDEX mentor_detail_without_papers ON mentor_detail (mentor_id)
 WHERE coalesce(json_array_length(papers), 0) = 0;
