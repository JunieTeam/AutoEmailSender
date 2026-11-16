-- The numbers of the links the model gave people as their pages, as it gave them, on a page it
-- read for lists; none on a page it did not read so. A page a link too far reading alike follows
-- only these, and leads elsewhere only where one of them does.
ALTER TABLE crawl_page ADD COLUMN given_links TEXT
  CHECK (given_links IS NULL OR (json_valid(given_links) AND json_type(given_links) = 'array'));
