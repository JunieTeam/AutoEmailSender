-- Custom addresses saved with the `/chat/completions` that calls append, as `model::Source::check`
-- now takes them: an edit that leaves the key blank finds the saved key by the address as checked.
UPDATE model_config
SET base_url = substr(base_url, 1, length(base_url) - length('/chat/completions'))
WHERE provider = 'custom'
  AND substr(base_url, -length('/chat/completions')) = '/chat/completions';
