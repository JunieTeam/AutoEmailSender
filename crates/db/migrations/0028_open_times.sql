-- The times of an email's latest opens as the server keeps them, newest first
-- (cloud_api::tracking::Open::times); replaced whole whenever its opens come.
ALTER TABLE email ADD COLUMN open_times TEXT NOT NULL DEFAULT '[]'
  CHECK (json_valid(open_times) AND json_type(open_times) = 'array');
