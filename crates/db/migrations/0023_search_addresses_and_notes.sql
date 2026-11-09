-- The keyword index now also holds addresses and notes. Tokens are made in Rust, so the index is
-- emptied here and `mentor::index_missing` writes it again once the app has started.
INSERT INTO mentor_search (mentor_search) VALUES ('delete-all');
