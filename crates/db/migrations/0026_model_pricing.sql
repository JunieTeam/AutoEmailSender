-- Model prices in yuan or dollars with a cache-hit price, off-peak prices for providers with peak
-- hours, and the region of a provider with several (docs/data-model.md#个人资料和配置).
ALTER TABLE model_config ADD COLUMN region TEXT;
ALTER TABLE model_config ADD COLUMN currency TEXT NOT NULL DEFAULT 'cny'
  CHECK (currency IN ('cny', 'usd'));
ALTER TABLE model_config ADD COLUMN cache_price REAL CHECK (cache_price >= 0);
ALTER TABLE model_config ADD COLUMN off_peak_input_price REAL CHECK (off_peak_input_price >= 0);
ALTER TABLE model_config ADD COLUMN off_peak_cache_price REAL CHECK (off_peak_cache_price >= 0);
ALTER TABLE model_config ADD COLUMN off_peak_output_price REAL CHECK (off_peak_output_price >= 0);
-- Bailian was called in Beijing before its region could be chosen.
UPDATE model_config SET region = 'cn-beijing' WHERE provider = 'qwen';

-- Whether a call fell in its provider's off-peak hours, which charge the off-peak prices.
ALTER TABLE usage ADD COLUMN off_peak INTEGER NOT NULL DEFAULT 0 CHECK (off_peak IN (0, 1));
-- DeepSeek's peak hours are 9:00–12:00 and 14:00–18:00 Beijing time (UTC+8), Monday to Friday;
-- 1970-01-01 was a Thursday.
UPDATE usage SET off_peak = 1
WHERE model_config_id IN (SELECT id FROM model_config WHERE provider = 'deepseek')
  AND NOT (((at / 1000 + 28800) / 86400 + 4) % 7 BETWEEN 1 AND 5
    AND (at / 1000 + 28800) % 86400 / 3600 IN (9, 10, 11, 14, 15, 16, 17));
