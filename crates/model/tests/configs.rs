// Marks the whole file as test code, so clippy's test allowances cover the helpers too.
#![cfg(test)]

use model::pricing::{Currency, Hours, Prices, Rate};
use model::{ModelForm, Source};
use sqlx::SqlitePool;

fn form(provider: &str, base_url: &str) -> ModelForm {
    ModelForm {
        name: String::new(),
        provider: provider.into(),
        region: None,
        base_url: base_url.into(),
        model: " deepseek-chat ".into(),
        currency: None,
        prices: Prices::default(),
        off_peak: Prices::default(),
    }
}

#[test]
fn a_key_pasted_with_more_than_the_key_is_refused() {
    assert_eq!(
        model::checked_key(" sk-ab\ncd ").unwrap().as_deref(),
        Some("sk-abcd")
    );
    assert!(model::checked_key("密钥 sk-abcd").is_err());
    assert_eq!(model::checked_key(" ").unwrap(), None);
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_saved_key_is_only_for_where_it_was_saved(db: SqlitePool) {
    let fields = form("deepseek", "").check().unwrap();
    let id = model::create(&db, &fields, "sk-deepseek", 0).await.unwrap();
    let key = async |source| model::api_key(&db, id, &source).await.unwrap();

    assert_eq!(
        key(Source::check("deepseek", None, "").unwrap())
            .await
            .flatten()
            .as_deref(),
        Some("sk-deepseek")
    );
    assert_eq!(key(Source::check("openai", None, "").unwrap()).await, None);
    let elsewhere = Source::check("custom", None, "https://api.example.com/v1").unwrap();
    assert_eq!(key(elsewhere).await, None);
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn the_earliest_model_is_the_default_until_another_is_chosen(db: SqlitePool) {
    let fields = form("deepseek", "").check().unwrap();
    let first = model::create(&db, &fields, "k", 0).await.unwrap();
    let second = model::create(&db, &fields, "k", 0).await.unwrap();
    let default = async || {
        let configs = model::list(&db, 0).await.unwrap();
        let ids: Vec<_> = configs
            .iter()
            .filter(|c| c.is_default)
            .map(|c| c.id)
            .collect();
        ids
    };
    assert_eq!(default().await, [first]);

    assert!(profile::set_default_model(&db, second).await.unwrap());
    assert_eq!(default().await, [second]);
    model::delete(&db, second).await.unwrap();
    assert_eq!(default().await, [first]);
    assert!(!profile::set_default_model(&db, second).await.unwrap());
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn what_the_user_agreed_to_holds_only_for_the_same_provider(db: SqlitePool) {
    let deepseek = form("deepseek", "").check().unwrap();
    let id = model::create(&db, &deepseek, "k", 0).await.unwrap();
    let agreed = async || model::chosen(&db).await.unwrap().unwrap().agreed;
    assert!(!agreed().await);

    // Not to a provider the user was not told about.
    let custom = Source::check("custom", None, "https://api.example.com/v1").unwrap();
    assert!(!model::agree(&db, id, &custom).await.unwrap());
    assert!(!agreed().await);
    assert!(model::agree(&db, id, &deepseek.source).await.unwrap());
    assert!(agreed().await);
    // Another model of the same provider was agreed to as well.
    let mut renamed = form("deepseek", "");
    renamed.model = "deepseek-reasoner".into();
    model::update(&db, id, &renamed.check().unwrap(), "k")
        .await
        .unwrap();
    assert!(agreed().await);
    let elsewhere = form("custom", "https://api.example.com/v1")
        .check()
        .unwrap();
    model::update(&db, id, &elsewhere, "k").await.unwrap();
    assert!(!agreed().await);
}

#[sqlx::test(migrations = false)]
async fn an_address_saved_with_chat_completions_keeps_its_key_after_the_upgrade(db: SqlitePool) {
    db::MIGRATOR.run_to(21, &db).await.unwrap();
    sqlx::query(
        "INSERT INTO model_config (id, provider, base_url, model, api_key, agreed, created_at)
        VALUES (1, 'custom', 'https://relay.example/v1/chat/completions', 'm', 'sk-relay', 1, 0)",
    )
    .execute(&db)
    .await
    .unwrap();
    db::MIGRATOR.run(&db).await.unwrap();

    let source =
        Source::check("custom", None, "https://relay.example/v1/chat/completions").unwrap();
    assert_eq!(
        model::api_key(&db, 1, &source).await.unwrap(),
        Some(Some("sk-relay".into()))
    );
    assert!(model::chosen(&db).await.unwrap().unwrap().agreed);
}

#[test]
fn a_custom_address_must_be_a_web_address() {
    let source = Source::check("custom", None, " https://api.example.com/v1/ ").unwrap();
    assert_eq!(
        source.access(Some("k".into())).base_url,
        "https://api.example.com/v1"
    );
    for api in ["chat/completions", "responses"] {
        let pasted =
            Source::check("custom", None, &format!("https://api.example.com/v1/{api}")).unwrap();
        assert_eq!(pasted.access(None).base_url, "https://api.example.com/v1");
    }
    assert!(Source::check("custom", None, "api.example.com/v1").is_err());
    for secret in [
        "https://user:pass@api.example.com/v1",
        "https://user@api.example.com/v1",
        "https://api.example.com/v1?api_key=k",
        "https://api.example.com/v1#k",
    ] {
        assert!(Source::check("custom", None, secret).is_err(), "{secret}");
    }
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn missing_key_is_distinct_from_a_keyless_model_and_pauses_background_work(db: SqlitePool) {
    let fields = form("custom", "http://127.0.0.1:1/v1").check().unwrap();
    let id = model::create(&db, &fields, "", 0).await.unwrap();
    assert!(!model::list(&db, 0).await.unwrap()[0].needs_api_key);
    assert_eq!(
        model::chosen(&db).await.unwrap().unwrap().access.api_key,
        Some(String::new())
    );
    sqlx::query("UPDATE model_config SET api_key = NULL WHERE id = ?")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    assert!(model::list(&db, 0).await.unwrap()[0].needs_api_key);
    let chosen = model::chosen(&db).await.unwrap().unwrap();
    let net = net::Net::new(vec![]).unwrap();
    let error = model::ask(&net, &chosen.access, &chosen.model, "system", "user", false)
        .await
        .unwrap_err();
    assert!(matches!(error, model::ModelError::MissingKey));
    assert!(error.failure().message.contains("设置 › AI › 模型"));
    assert!(matches!(
        model::background::classify(error, &chosen.access),
        model::background::Effect::Pause(_)
    ));
    assert!(matches!(
        model::list_models(&net, &chosen.access).await,
        Err(model::ModelError::MissingKey)
    ));
    assert!(matches!(
        model::test(&net, &chosen.access, &chosen.model).await,
        Err(model::ModelError::MissingKey)
    ));
}

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn agreement_and_key_hold_only_in_the_same_region(db: SqlitePool) {
    let fields = form("qwen", "").check().unwrap();
    let id = model::create(&db, &fields, "sk-beijing", 0).await.unwrap();
    // Left out, the region is the first, for the agreement as well.
    let configs = model::list(&db, 0).await.unwrap();
    assert_eq!(configs[0].region.as_deref(), Some("cn-beijing"));
    let beijing = Source::check("qwen", None, "").unwrap();
    assert!(model::agree(&db, id, &beijing).await.unwrap());

    let singapore = Source::check("qwen", Some("ap-southeast-1"), "").unwrap();
    assert_eq!(
        singapore.access(None).base_url,
        "https://dashscope-intl.aliyuncs.com/compatible-mode/v1"
    );
    assert_eq!(model::api_key(&db, id, &singapore).await.unwrap(), None);
    let mut moved = form("qwen", "");
    moved.region = Some("ap-southeast-1".into());
    model::update(&db, id, &moved.check().unwrap(), "sk-singapore")
        .await
        .unwrap();
    assert!(!model::chosen(&db).await.unwrap().unwrap().agreed);
    assert!(!model::agree(&db, id, &beijing).await.unwrap());
    assert!(model::agree(&db, id, &singapore).await.unwrap());
    assert!(Source::check("qwen", Some("eu-central-1"), "").is_err());
}

#[test]
fn prices_are_in_the_providers_currency_unless_chosen_and_off_peak_ones_need_peak_hours() {
    let currency =
        |provider: &str, base_url: &str| form(provider, base_url).check().unwrap().pricing.currency;
    assert_eq!(currency("openai", ""), Currency::Usd);
    assert_eq!(currency("deepseek", ""), Currency::Cny);
    assert_eq!(
        currency("custom", "https://api.example.com/v1"),
        Currency::Cny
    );
    let mut chosen = form("openai", "");
    chosen.currency = Some(Currency::Cny);
    assert_eq!(chosen.check().unwrap().pricing.currency, Currency::Cny);

    for (provider, accepted) in [("deepseek", true), ("kimi", false)] {
        let mut f = form(provider, "");
        f.off_peak = Prices::official(1.0, None, 4.0);
        assert_eq!(f.check().is_ok(), accepted, "{provider}");
    }
    let mut negative = form("kimi", "");
    negative.prices.cache = Some(-1.0);
    assert!(negative.check().is_err());
}

/// 2026-11-10, a Tuesday, at 10:00 Beijing time: DeepSeek's peak hours.
const TUESDAY_MORNING: i64 = 1_794_276_000_000;
/// 12:30, between its two peaks.
const TUESDAY_NOON: i64 = 1_794_285_000_000;
/// 18:00, when the second ends.
const TUESDAY_EVENING: i64 = 1_794_304_800_000;
/// 2026-11-14, a Saturday, at 10:00.
const SATURDAY_MORNING: i64 = 1_794_621_600_000;

#[sqlx::test(migrator = "db::MIGRATOR")]
async fn a_call_is_charged_the_prices_of_the_hours_it_is_made_in(db: SqlitePool) {
    const PEAK: Prices = Prices::official(2.0, Some(0.04), 8.0);
    const OFF_PEAK: Prices = Prices::official(1.0, Some(0.02), 4.0);
    let deepseek = |prices, off_peak| {
        let mut f = form("deepseek", "");
        f.prices = prices;
        f.off_peak = off_peak;
        f.check().unwrap()
    };
    let id = model::create(&db, &deepseek(PEAK, OFF_PEAK), "k", 0)
        .await
        .unwrap();
    let mut kimi = form("kimi", "");
    kimi.prices = PEAK;
    model::create(&db, &kimi.check().unwrap(), "k", 0)
        .await
        .unwrap();
    let rate = async |at| model::list(&db, at).await.unwrap()[0].rate;
    let peak = Rate {
        input: 2.0,
        output: 8.0,
        currency: Currency::Cny,
        hours: Some(Hours::Peak),
    };
    let off_peak = Rate {
        input: 1.0,
        output: 4.0,
        hours: Some(Hours::OffPeak),
        ..peak
    };
    assert_eq!(rate(TUESDAY_MORNING).await, Some(peak));
    for at in [TUESDAY_NOON, TUESDAY_EVENING, SATURDAY_MORNING] {
        assert_eq!(rate(at).await, Some(off_peak), "{at}");
    }
    // A provider charging the same at all hours has no hours to tell.
    let others = model::list(&db, SATURDAY_MORNING).await.unwrap();
    assert_eq!(
        others[1].rate,
        Some(Rate {
            hours: None,
            ..peak
        })
    );

    // Without both off-peak prices, the peak ones stand.
    let incomplete = Prices {
        output: None,
        ..OFF_PEAK
    };
    model::update(&db, id, &deepseek(PEAK, incomplete), "k")
        .await
        .unwrap();
    assert_eq!(rate(SATURDAY_MORNING).await, Some(peak));
    let unpriced = Prices {
        input: None,
        ..PEAK
    };
    model::update(&db, id, &deepseek(unpriced, OFF_PEAK), "k")
        .await
        .unwrap();
    assert_eq!(rate(TUESDAY_MORNING).await, None);
    assert_eq!(rate(SATURDAY_MORNING).await, Some(off_peak));
}

#[sqlx::test(migrations = false)]
async fn bailian_keeps_its_key_and_deepseek_calls_learn_their_hours_on_the_upgrade(db: SqlitePool) {
    db::MIGRATOR.run_to(25, &db).await.unwrap();
    sqlx::query(
        "INSERT INTO model_config (id, provider, model, api_key, agreed, created_at)
        VALUES (1, 'qwen', 'qwen3.8-max', 'sk-qwen', 1, 0),
            (2, 'deepseek', 'deepseek-flash', 'sk-deepseek', 1, 0)",
    )
    .execute(&db)
    .await
    .unwrap();
    for (config, at) in [
        (2, TUESDAY_MORNING),
        (2, TUESDAY_NOON),
        (2, TUESDAY_EVENING),
        (2, SATURDAY_MORNING),
        (1, SATURDAY_MORNING),
    ] {
        sqlx::query(
            "INSERT INTO usage (at, model_config_id, model, purpose, input_tokens, cached_tokens,
                output_tokens)
            VALUES (?, ?, 'm', 'match', 1, 0, 1)",
        )
        .bind(at)
        .bind(config)
        .execute(&db)
        .await
        .unwrap();
    }
    db::MIGRATOR.run(&db).await.unwrap();

    let beijing = Source::check("qwen", None, "").unwrap();
    assert_eq!(
        model::api_key(&db, 1, &beijing).await.unwrap(),
        Some(Some("sk-qwen".into()))
    );
    assert!(model::selected(&db, Some(1)).await.unwrap().unwrap().agreed);
    assert_eq!(
        model::list(&db, 0).await.unwrap()[0].pricing.currency,
        Currency::Cny
    );
    let off_peak: Vec<bool> = sqlx::query_scalar("SELECT off_peak FROM usage ORDER BY id")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(off_peak, [false, true, true, true, false]);
}
