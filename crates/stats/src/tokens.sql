SELECT u.purpose AS "purpose!: model::usage::Purpose", u.off_peak AS "off_peak!: bool",
 c.currency AS "currency?: model::pricing::Currency", c.input_price, c.cache_price, c.output_price,
 c.off_peak_input_price, c.off_peak_cache_price, c.off_peak_output_price,
 sum(u.input_tokens) AS "input!: i64", sum(u.cached_tokens) AS "cached!: i64",
 sum(u.output_tokens) AS "output!: i64", count(*) AS "calls!: i64"
FROM usage u LEFT JOIN model_config c ON c.id=u.model_config_id
WHERE (?1 IS NULL OR u.at>=?1) AND u.at<?2
GROUP BY u.purpose, u.model_config_id, u.off_peak ORDER BY u.purpose
