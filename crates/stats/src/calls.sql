SELECT u.id, u.at, u.purpose AS "purpose!: model::usage::Purpose", u.model, u.crawl_id,
 u.input_tokens, u.cached_tokens, u.output_tokens, u.off_peak AS "off_peak!: bool",
 u.error AS "error?: Json<Failure>", m.id AS "mentor_id?", m.name AS "mentor_name?",
 c.currency AS "currency?: model::pricing::Currency", c.input_price, c.cache_price, c.output_price,
 c.off_peak_input_price, c.off_peak_cache_price, c.off_peak_output_price
FROM usage u
LEFT JOIN model_config c ON c.id=u.model_config_id
LEFT JOIN mentor m ON m.id=u.mentor_id
WHERE (?1 IS NULL OR u.at>=?1) AND u.at<?2 AND (?3 IS NULL OR u.purpose=?3)
 AND (?4 IS NULL OR u.model=?4)
ORDER BY u.at DESC, u.id DESC
LIMIT ?5 OFFSET ?6
