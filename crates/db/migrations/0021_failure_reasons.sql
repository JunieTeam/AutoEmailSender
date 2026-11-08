-- Reasons shown to the user become a `Failure`: the message, and the outside words the app did
-- not recognize (docs/data-model.md#通用约定). Earlier reasons have no such words of their own.
UPDATE email SET error = json_object('message', error, 'details', NULL) WHERE error IS NOT NULL;
UPDATE email
  SET fact_check = json_set(fact_check, '$.reason',
    json_object('message', fact_check ->> '$.reason', 'details', NULL))
  WHERE fact_check ->> '$.state' = 'failed';
UPDATE message_mentor SET bounce_reason = json_object('message', bounce_reason, 'details', NULL)
  WHERE bounce_reason IS NOT NULL;
UPDATE mailbox SET pause_reason = json_object('message', pause_reason, 'details', NULL)
  WHERE pause_reason IS NOT NULL;
UPDATE mailbox SET receive_error = json_object('message', receive_error, 'details', NULL)
  WHERE receive_error IS NOT NULL;
UPDATE mailbox
  SET cloud_pause_reason = json_object('message', cloud_pause_reason, 'details', NULL)
  WHERE cloud_pause_reason IS NOT NULL;
UPDATE mentor SET match_error = json_object('message', match_error, 'details', NULL)
  WHERE match_error IS NOT NULL;
UPDATE profile
  SET match_pause_reason = json_object('message', match_pause_reason, 'details', NULL)
  WHERE match_pause_reason IS NOT NULL;
UPDATE profile
  SET reply_processing_pause_reason =
    json_object('message', reply_processing_pause_reason, 'details', NULL)
  WHERE reply_processing_pause_reason IS NOT NULL;
UPDATE profile
  SET cloud_backup_error = json_object('message', cloud_backup_error, 'details', NULL)
  WHERE cloud_backup_error IS NOT NULL;
UPDATE task
  SET generation_pause_reason = json_object('message', generation_pause_reason, 'details', NULL)
  WHERE generation_pause_reason IS NOT NULL;
UPDATE reply_processing SET error = json_object('message', error, 'details', NULL)
  WHERE error IS NOT NULL;
UPDATE crawl SET error = json_object('message', error, 'details', NULL) WHERE error IS NOT NULL;
UPDATE crawl_page SET error = json_object('message', error, 'details', NULL)
  WHERE error IS NOT NULL;
UPDATE candidate SET enrich_error = json_object('message', enrich_error, 'details', NULL)
  WHERE enrich_error IS NOT NULL;
