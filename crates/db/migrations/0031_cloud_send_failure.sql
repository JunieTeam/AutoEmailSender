-- Since when synchronizing cloud sending with the official server has been failing, and the
-- latest reason; cleared by the next success.
ALTER TABLE account ADD COLUMN cloud_send_failed_at INTEGER;
ALTER TABLE account ADD COLUMN cloud_send_error TEXT
  CHECK ((cloud_send_error IS NULL) = (cloud_send_failed_at IS NULL));
