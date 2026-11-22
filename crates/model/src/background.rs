//! Shared model capacity and failure policy for persisted background queues.

use crate::error::{Cause, NO_BALANCE};
use crate::{Access, Chosen, ModelError};
use failure::Failure;
use std::{
    collections::{BTreeSet, HashMap},
    future::Future,
    hash::Hash,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{sync::Notify, time::Instant};

/// The calls to one model of an account running at once, and the items of each background queue.
pub const CONCURRENCY: usize = 3;

/// Rate limits or network failures lasting longer than this with nothing getting through are
/// taken as ones that will not lift by themselves, such as a used-up plan, and pause the work.
pub const LASTING: Duration = Duration::from_secs(10 * 60);

/// Which waiting call goes first: the user's, waited for on the screen, before background work's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Interactive,
    Background,
}

/// After a rate limit, a group gets back one more call at once for each this many answers without
/// another.
const STEADY: usize = 5;

/// The calls to models running at once (docs/development.md#异步和后台任务), apart by the address,
/// API key and model they go to, as providers limit an account's calls to a model. One for the app.
#[derive(Clone, Default)]
pub struct Turns(Arc<Lines>);

#[derive(Default)]
struct Lines {
    lines: Mutex<HashMap<Group, Line>>,
    /// A turn may have come free for someone waiting.
    freed: Notify,
    tickets: AtomicU64,
}

/// The address, API key and model.
type Group = (String, Option<String>, String);

fn group(access: &Access, model: &str) -> Group {
    (
        access.base_url.clone(),
        access.api_key.clone(),
        model.to_owned(),
    )
}

struct Line {
    /// How many calls may run at once.
    allowed: usize,
    running: usize,
    /// Answers since the last rate limit or the last call added back.
    answered: usize,
    /// The calls waiting, in the order they go.
    waiting: BTreeSet<(Priority, u64)>,
}
impl Default for Line {
    fn default() -> Self {
        Self {
            allowed: CONCURRENCY,
            running: 0,
            answered: 0,
            waiting: BTreeSet::new(),
        }
    }
}

impl Turns {
    /// A turn to call `model` through `access`, once fewer calls than its group allows are running
    /// and none waits before it.
    pub async fn take(&self, access: &Access, model: &str, priority: Priority) -> Turn {
        let group = group(access, model);
        let ticket = (priority, self.0.tickets.fetch_add(1, Ordering::Relaxed));
        self.lines()
            .entry(group.clone())
            .or_default()
            .waiting
            .insert(ticket);
        let queued = Queued {
            turns: self,
            group: &group,
            ticket,
        };
        loop {
            // Made before looking, so that it hears a turn coming free in between.
            let freed = self.0.freed.notified();
            {
                let mut lines = self.lines();
                let line = line(&mut lines, &group);
                if line.running < line.allowed && line.waiting.first() == Some(&ticket) {
                    line.waiting.pop_first();
                    line.running += 1;
                    self.wake(line);
                    break;
                }
            }
            freed.await;
        }
        drop(queued);
        Turn {
            turns: self.clone(),
            access: access.clone(),
            model: model.to_owned(),
        }
    }

    /// A turn at the configuration `read` gives, as it is once the turn comes, and calling as it
    /// says then: one changed to another group meanwhile waits again there. None without a
    /// configuration.
    pub async fn take_current<E, F>(
        &self,
        priority: Priority,
        read: impl Fn() -> F,
    ) -> Result<Option<(Turn, Chosen)>, E>
    where
        F: Future<Output = Result<Option<Chosen>, E>>,
    {
        let Some(mut chosen) = read().await? else {
            return Ok(None);
        };
        loop {
            let mut turn = self.take(&chosen.access, &chosen.model, priority).await;
            match read().await? {
                Some(current) if turn.serves(&current) => {
                    // The same group, but how it is asked follows the provider, which may not be.
                    turn.access = current.access.clone();
                    return Ok(Some((turn, current)));
                }
                Some(current) => chosen = current,
                None => return Ok(None),
            }
        }
    }

    /// Lets the calls waiting look again when one of them can go.
    fn wake(&self, line: &Line) {
        if line.running < line.allowed && !line.waiting.is_empty() {
            self.0.freed.notify_waiters();
        }
    }

    fn lines(&self) -> MutexGuard<'_, HashMap<Group, Line>> {
        self.0.lines.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[expect(clippy::unreachable, reason = "a group's line stays once added")]
fn line<'a>(lines: &'a mut HashMap<Group, Line>, group: &Group) -> &'a mut Line {
    lines
        .get_mut(group)
        .unwrap_or_else(|| unreachable!("a group's line stays once added"))
}

/// A call's place in line until it gets its turn or stops waiting.
struct Queued<'a> {
    turns: &'a Turns,
    group: &'a Group,
    ticket: (Priority, u64),
}
impl Drop for Queued<'_> {
    fn drop(&mut self) {
        let mut lines = self.turns.lines();
        let line = line(&mut lines, self.group);
        if line.waiting.remove(&self.ticket) {
            self.turns.wake(line);
        }
    }
}

/// One of the calls a group may run at once, until dropped.
pub struct Turn {
    turns: Turns,
    access: Access,
    model: String,
}
impl Turn {
    pub fn access(&self) -> &Access {
        &self.access
    }
    pub fn model(&self) -> &str {
        &self.model
    }
    /// Whether `chosen` calls the group this is a turn of.
    pub fn serves(&self, chosen: &Chosen) -> bool {
        chosen.model == self.model
            && chosen.access.base_url == self.access.base_url
            && chosen.access.api_key == self.access.api_key
    }
    /// Notes how a call in this turn went: a rate limit takes the group down to one call at once,
    /// and answers without another bring it back up.
    pub(crate) fn note<T>(&self, result: &Result<T, ModelError>) {
        let limited = match result {
            Ok(_) => false,
            // A network that fails says nothing of the provider's limits.
            Err(ModelError::Unreachable { .. } | ModelError::TimedOut { .. }) => return,
            Err(error) if matches!(classify(error, &self.access), Effect::RateLimit(_)) => true,
            Err(_) => return,
        };
        let mut lines = self.turns.lines();
        let line = line(&mut lines, &group(&self.access, &self.model));
        if limited {
            line.allowed = 1;
            line.answered = 0;
        } else if line.allowed < CONCURRENCY {
            line.answered += 1;
            if line.answered == STEADY {
                line.allowed += 1;
                line.answered = 0;
                self.turns.wake(line);
            }
        }
    }
}
impl Drop for Turn {
    fn drop(&mut self) {
        let mut lines = self.turns.lines();
        let line = line(&mut lines, &group(&self.access, &self.model));
        line.running -= 1;
        self.turns.wake(line);
    }
}

pub fn same_config(current: &Chosen, expected: &Chosen) -> bool {
    current.id == expected.id
        && current.model == expected.model
        && current.access.base_url == expected.access.base_url
        && current.access.api_key == expected.access.api_key
}

/// What a failure means for background work: pause all of it, wait out a rate limit or the
/// network being down, or fail only this item.
pub enum Effect {
    Pause(Failure),
    RateLimit(RateLimit),
    Failed(Failure),
}

#[derive(Debug)]
pub struct RateLimit {
    pub wait: Duration,
    /// The reason to pause with when the limits last.
    pub lasting: Failure,
}

/// How many times the user has started or resumed each piece of background work (`K`): rate
/// limits met in an earlier round, even by calls still running when it ended, do not count toward
/// pausing it.
#[derive(Default)]
pub struct Rounds<K> {
    rounds: Mutex<HashMap<K, u64>>,
    /// A round began, which may lift a cooldown being waited out.
    began: Notify,
}
impl<K: Eq + Hash> Rounds<K> {
    pub fn next(&self, key: K) {
        *self.rounds().entry(key).or_default() += 1;
        self.began.notify_waiters();
    }
    pub fn current(&self, key: &K) -> u64 {
        self.rounds().get(key).copied().unwrap_or_default()
    }
    fn rounds(&self) -> MutexGuard<'_, HashMap<K, u64>> {
        self.rounds.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Until when rate limits hold background work back. Each limit belongs to the piece of work
/// (`K`) and round whose call met it: once the user goes on with a piece, what it met before no
/// longer holds it back, while what other pieces met still does.
pub struct Cooldown<K> {
    rounds: Arc<Rounds<K>>,
    until: Arc<Mutex<HashMap<K, (u64, Instant)>>>,
}
impl<K> Clone for Cooldown<K> {
    fn clone(&self) -> Self {
        Self {
            rounds: self.rounds.clone(),
            until: self.until.clone(),
        }
    }
}
impl<K: Eq + Hash> Cooldown<K> {
    pub fn new(rounds: Arc<Rounds<K>>) -> Self {
        Self {
            rounds,
            until: Arc::default(),
        }
    }
    /// The round of `key` a call going out now belongs to.
    pub fn round(&self, key: &K) -> u64 {
        self.rounds.current(key)
    }
    /// When the last limit still holding back ends.
    pub fn until(&self) -> Option<Instant> {
        let now = Instant::now();
        let mut until = self.lock();
        until.retain(|key, &mut (round, at)| round == self.rounds.current(key) && at > now);
        until.values().map(|&(_, at)| at).max()
    }
    /// Holds back for `wait` from now, for a limit a call of `key` made in `round` met.
    pub fn defer(&self, key: K, round: u64, wait: Duration) {
        let at = Instant::now() + wait;
        let mut until = self.lock();
        let held = until.entry(key).or_insert((round, at));
        *held = if held.0 == round {
            (round, held.1.max(at))
        } else {
            (round, at)
        };
    }
    /// A turn at `chosen`'s model, once no limit holds back.
    pub async fn acquire(&self, turns: &Turns, chosen: &Chosen) -> Turn {
        loop {
            // Made before looking, so that it hears a round that begins in between.
            let began = self.rounds.began.notified();
            if let Some(at) = self.until() {
                tokio::select! {
                    () = tokio::time::sleep_until(at) => {}
                    () = began => {}
                }
                continue;
            }
            let turn = turns
                .take(&chosen.access, &chosen.model, Priority::Background)
                .await;
            if self.until().is_none() {
                return turn;
            }
        }
    }
    fn lock(&self) -> MutexGuard<'_, HashMap<K, (u64, Instant)>> {
        self.until.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Rate limits one piece of background work has met in a round, with nothing getting through in
/// between. Whoever keeps it starts it afresh once something gets through, so that only time spent
/// waiting out limits counts.
#[derive(Default)]
pub struct Limited {
    round: u64,
    /// By configuration: work can run on two at once, and late answers can come through one the
    /// user has replaced since.
    waits: Vec<Waits>,
}
struct Waits {
    config: Chosen,
    /// Each stretch of time counted once however many limits asked for it.
    blocked: Duration,
    until: Instant,
}
impl Limited {
    /// Notes a limit met `now` through `config` that asks to wait `wait`, by a call made in the
    /// current `round`; true once the waiting through `config` is longer than `LASTING`.
    pub fn lasting(&mut self, round: u64, now: Instant, wait: Duration, config: &Chosen) -> bool {
        if self.round != round {
            *self = Limited {
                round,
                waits: Vec::new(),
            };
        }
        let i = self
            .waits
            .iter()
            .position(|w| same_config(&w.config, config))
            .unwrap_or_else(|| {
                self.waits.push(Waits {
                    config: config.clone(),
                    blocked: Duration::ZERO,
                    until: now,
                });
                self.waits.len() - 1
            });
        let w = &mut self.waits[i];
        let end = now + wait;
        if end > w.until {
            w.blocked += end - w.until.max(now);
            w.until = end;
        }
        w.blocked > LASTING
    }
}

/// How background work goes on after `error` from calling through `access`.
pub fn classify(error: &ModelError, access: &Access) -> Effect {
    let failure = error.failure();
    let (cause, status, retry_after, words) = match error {
        ModelError::MissingKey => return Effect::Pause(failure),
        ModelError::Unreachable { source, .. } if net::untrusted(source) => {
            return Effect::Pause(failure);
        }
        // A network that dropped or a provider that stalled often comes back by itself: waited out
        // like a rate limit, a short outage does not stop work left running unattended.
        ModelError::Unreachable { .. } | ModelError::TimedOut { .. } => {
            return Effect::RateLimit(RateLimit {
                wait: Duration::from_secs(60),
                lasting: failure,
            });
        }
        ModelError::Refused {
            status,
            reply,
            retry_after,
            code,
        } => (
            Cause::of(*status, code.as_deref(), reply),
            Some(*status),
            *retry_after,
            reply,
        ),
        ModelError::Failed(reply) => (Cause::of(0, None, reply), None, None, reply),
        ModelError::NoAnswer { .. } | ModelError::Broken(_) => return Effect::Failed(failure),
    };
    let rate_limited =
        status == Some(429) && access.provider.is_some_and(|p| p.quota_is_rate_limit);
    match cause {
        Cause::TooLong | Cause::ServerError | Cause::Other => Effect::Failed(failure),
        Cause::Quota if !rate_limited => Effect::Pause(Failure::new(NO_BALANCE)),
        Cause::NoBalance | Cause::NoModel | Cause::NotOrdered | Cause::BadKey | Cause::NotFound => {
            Effect::Pause(failure)
        }
        Cause::Quota | Cause::RateLimited => Effect::RateLimit(RateLimit {
            // A provider asking for no wait at all is still limiting; a second keeps retries from
            // running back to back.
            wait: retry_after
                .unwrap_or(Duration::from_secs(30))
                .max(Duration::from_secs(1)),
            lasting: Failure::unrecognized(
                format!(
                    "模型服务商限流超过 {} 分钟。可能是套餐到期或用量到了上限，到服务商网站查看，\
                    处理好后继续。",
                    LASTING.as_secs() / 60
                ),
                words,
            ),
        }),
    }
}

/// Approximate tokens from CJK characters and groups of four other characters.
pub fn tokens(text: &str) -> i64 {
    let (mut cjk, mut other) = (0, 0);
    for c in text.chars() {
        if matches!(
            u32::from(c),
            0x2e80..=0x9fff | 0xac00..=0xd7af | 0xf900..=0xfaff | 0x20000..=0x323af
        ) {
            cjk += 1;
        } else {
            other += 1;
        }
    }
    cjk + (other + 3) / 4
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Source;

    fn chosen(key: &str, model: &str) -> Chosen {
        Chosen {
            id: 1,
            provider: "glm".into(),
            model: model.into(),
            agreed: true,
            access: Source::check("glm", None, "https://relay.example.com/v1")
                .unwrap()
                .access(Some(key.into())),
        }
    }

    /// What `take` gives without waiting.
    async fn at_once<T>(take: impl Future<Output = T>) -> Option<T> {
        tokio::select! {
            biased;
            taken = take => Some(taken),
            () = std::future::ready(()) => None,
        }
    }

    #[tokio::test]
    async fn each_model_of_an_account_runs_its_calls_apart_from_the_others() {
        let turns = Turns::default();
        let (a, b) = (chosen("a", "m").access, chosen("b", "m").access);
        let mut running = Vec::new();
        for _ in 0..CONCURRENCY {
            running.push(turns.take(&a, "m", Priority::Background).await);
        }
        assert!(
            at_once(turns.take(&a, "m", Priority::Background))
                .await
                .is_none()
        );
        assert!(
            at_once(turns.take(&a, "n", Priority::Background))
                .await
                .is_some()
        );
        assert!(
            at_once(turns.take(&b, "m", Priority::Background))
                .await
                .is_some()
        );
        running.pop();
        assert!(
            at_once(turns.take(&a, "m", Priority::Background))
                .await
                .is_some()
        );
    }

    #[tokio::test]
    async fn a_call_the_user_waits_for_goes_before_background_work_waiting_longer() {
        let turns = Turns::default();
        let a = chosen("a", "m").access;
        let mut running = Vec::new();
        for _ in 0..CONCURRENCY {
            running.push(turns.take(&a, "m", Priority::Background).await);
        }
        let waiting = |priority| {
            let (turns, a) = (turns.clone(), a.clone());
            tokio::spawn(async move { turns.take(&a, "m", priority).await })
        };
        let background = waiting(Priority::Background);
        tokio::task::yield_now().await;
        let user = waiting(Priority::Interactive);
        tokio::task::yield_now().await;
        running.pop();
        let taken = user.await.unwrap();
        assert!(!background.is_finished());
        drop(taken);
        background.await.unwrap();
    }

    #[tokio::test]
    async fn a_rate_limit_leaves_one_call_at_once_until_answers_without_another_bring_more_back() {
        let turns = Turns::default();
        let a = chosen("a", "m").access;
        let take = || turns.take(&a, "m", Priority::Background);
        let limited: Result<(), ModelError> =
            Err(refused(429, "1302", "您当前使用该API的并发数过高"));
        take().await.note(&limited);
        let only = take().await;
        assert!(at_once(take()).await.is_none());
        drop(only);
        // A network failing in between neither limits nor counts.
        take().await.note(&Err::<(), _>(ModelError::TimedOut {
            address: "https://relay.example.com/v1".into(),
        }));
        for _ in 0..STEADY {
            take().await.note(&Ok(()));
        }
        let two = [take().await, take().await];
        assert!(at_once(take()).await.is_none());
        drop(two);
    }

    #[tokio::test]
    async fn a_call_whose_configuration_changes_while_waiting_waits_again_at_the_new_model() {
        let turns = Turns::default();
        let current = Mutex::new(chosen("a", "m"));
        let read = || async { Ok::<_, ()>(Some(current.lock().unwrap().clone())) };
        let mut running = Vec::new();
        for _ in 0..CONCURRENCY {
            running.push(
                turns
                    .take(&chosen("a", "m").access, "m", Priority::Background)
                    .await,
            );
        }
        let mut waiting = std::pin::pin!(turns.take_current(Priority::Background, read));
        assert!(at_once(&mut waiting).await.is_none());
        *current.lock().unwrap() = chosen("a", "n");
        running.pop();
        let (turn, taken) = waiting.await.unwrap().unwrap();
        assert_eq!((turn.model(), taken.model.as_str()), ("n", "n"));
        // It did not keep the turn it no longer needed there.
        assert!(
            at_once(turns.take(&chosen("a", "m").access, "m", Priority::Background))
                .await
                .is_some()
        );
    }

    #[tokio::test]
    async fn a_call_whose_provider_changes_while_waiting_goes_out_as_the_provider_now_is() {
        let turns = Turns::default();
        let glm = chosen("a", "m");
        // The same address, key and model, entered as an address of the user's own.
        let entered = Chosen {
            provider: crate::providers::CUSTOM.into(),
            access: Source::check(crate::providers::CUSTOM, None, &glm.access.base_url)
                .unwrap()
                .access(Some("a".into())),
            ..glm.clone()
        };
        let current = Mutex::new(entered);
        let read = || async { Ok::<_, ()>(Some(current.lock().unwrap().clone())) };
        let mut running = Vec::new();
        for _ in 0..CONCURRENCY {
            running.push(turns.take(&glm.access, "m", Priority::Background).await);
        }
        let mut waiting = std::pin::pin!(turns.take_current(Priority::Background, read));
        assert!(at_once(&mut waiting).await.is_none());
        *current.lock().unwrap() = glm;
        running.pop();
        let (turn, _) = waiting.await.unwrap().unwrap();
        assert!(turn.access().provider.is_some());
    }

    fn refused(status: u16, code: &str, reply: &str) -> ModelError {
        ModelError::Refused {
            status,
            reply: reply.into(),
            retry_after: None,
            code: Some(code.into()),
        }
    }

    #[test]
    fn rate_limits_pause_once_waiting_them_out_takes_over_ten_minutes() {
        let config = |key: &str| Chosen {
            id: 1,
            provider: "custom".into(),
            model: "m".into(),
            agreed: true,
            access: Source::check("custom", None, "https://relay.example.com/v1")
                .unwrap()
                .access(Some(key.into())),
        };
        let (a, b) = (config("k"), config("new key"));
        let start = Instant::now();
        let at = |secs| start + Duration::from_secs(secs);
        let secs = Duration::from_secs;

        // Retried as soon as each 30-second wait ends: the 21st limit passes ten minutes.
        let mut limited = Limited::default();
        let pauses: Vec<_> = (0..21)
            .map(|n| limited.lasting(0, at(n * 31), secs(30), &a))
            .collect();
        assert_eq!(pauses.iter().position(|&p| p), Some(20));

        assert!(!Limited::default().lasting(0, at(0), secs(600), &a));
        assert!(Limited::default().lasting(0, at(0), secs(601), &a));

        // Limits met together count their longest wait once.
        let mut limited = Limited::default();
        for wait in [590, 1, 0] {
            assert!(!limited.lasting(0, at(0), secs(wait), &a));
        }
        assert!(limited.lasting(0, at(591), secs(590), &a));

        // Waiting for a turn at the model between limits does not count.
        let mut limited = Limited::default();
        assert!(!limited.lasting(0, at(0), secs(300), &a));
        assert!(!limited.lasting(0, at(3600), secs(300), &a));

        // Each configuration counts its own waits, whichever answers last; a new round starts
        // afresh.
        let mut limited = Limited::default();
        assert!(!limited.lasting(0, at(0), secs(590), &b));
        assert!(!limited.lasting(0, at(1), secs(1), &a));
        assert!(!limited.lasting(0, at(591), secs(590), &a));
        assert!(limited.lasting(0, at(591), secs(30), &b));
        assert!(!limited.lasting(1, at(1200), secs(590), &b));
    }

    #[tokio::test(start_paused = true)]
    async fn going_on_with_one_piece_of_work_drops_only_the_limits_it_met_before() {
        let rounds = Arc::new(Rounds::default());
        let cooldown = Cooldown::new(rounds.clone());
        let secs = Duration::from_secs;
        let at = |wait| Instant::now() + secs(wait);
        cooldown.defer(1, 0, secs(590));
        cooldown.defer(1, 0, secs(10));
        cooldown.defer(2, 0, secs(60));
        assert_eq!(cooldown.until(), Some(at(590)));
        rounds.next(1);
        assert_eq!(cooldown.until(), Some(at(60)));
        cooldown.defer(1, 1, secs(90));
        assert_eq!(cooldown.until(), Some(at(90)));
        tokio::time::advance(secs(90)).await;
        assert_eq!(cooldown.until(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn a_call_waiting_out_a_limit_stops_waiting_once_going_on_drops_it() {
        let rounds = Arc::new(Rounds::default());
        let cooldown = Cooldown::new(rounds.clone());
        cooldown.defer(1, 0, Duration::from_secs(590));
        cooldown.defer(2, 0, Duration::from_secs(60));
        let start = Instant::now();
        let waiting = tokio::spawn({
            let cooldown = cooldown.clone();
            async move {
                let _turn = cooldown.acquire(&Turns::default(), &chosen("k", "m")).await;
                Instant::now()
            }
        });
        // Asleep until the 590 seconds are over.
        tokio::task::yield_now().await;
        rounds.next(1);
        // Woken, it still waits out the limit the other piece met.
        assert_eq!(waiting.await.unwrap(), start + Duration::from_secs(60));
    }

    #[test]
    fn a_used_up_balance_pauses_and_a_token_limit_waits() {
        let access = |provider| {
            Source::check(provider, None, "https://relay.example.com/v1")
                .unwrap()
                .access(Some("k".into()))
        };
        let quota = "You exceeded your current quota, please check your plan and billing details.";
        for (provider, error, expected) in [
            ("openai", refused(429, "insufficient_quota", quota), "pause"),
            ("custom", refused(429, "insufficient_quota", quota), "pause"),
            ("qwen", refused(429, "insufficient_quota", quota), "wait"),
            (
                "qwen",
                refused(
                    429,
                    "Throttling.AllocationQuota",
                    "Allocated quota exceeded, please increase your quota limit.",
                ),
                "wait",
            ),
            (
                "qwen",
                refused(
                    400,
                    "Arrearage",
                    "Access denied, please make sure your account is in good standing.",
                ),
                "pause",
            ),
            (
                "qwen",
                refused(429, "PrepaidBillOverdue", "The prepaid bill is overdue."),
                "pause",
            ),
            (
                "qwen",
                refused(
                    429,
                    "CommodityNotPurchased",
                    "Commodity has not purchased yet.",
                ),
                "pause",
            ),
            (
                "qwen",
                refused(
                    429,
                    "BudgetLimitExceeded",
                    "The budget configured in Budget Management has been exhausted.",
                ),
                "pause",
            ),
            (
                "doubao",
                refused(
                    429,
                    "SetLimitExceeded",
                    "Your account has reached the set inference limit for the model, and the model service has been paused.",
                ),
                "pause",
            ),
            (
                "doubao",
                refused(
                    429,
                    "QuotaExceeded",
                    "Your account has exhausted its free trial quota for the model.",
                ),
                "pause",
            ),
            (
                "doubao",
                refused(
                    429,
                    "RateLimitExceeded.EndpointTPMExceeded",
                    "TPM limit exceeded",
                ),
                "wait",
            ),
            (
                "deepseek",
                refused(402, "invalid_request_error", "Insufficient Balance"),
                "pause",
            ),
            (
                "glm",
                refused(429, "1113", "余额不足或无可用资源包,请充值。"),
                "pause",
            ),
            (
                "glm",
                refused(429, "1302", "您当前使用该API的并发数过高"),
                "wait",
            ),
        ] {
            let got = match classify(&error, &access(provider)) {
                Effect::Pause(_) => "pause",
                Effect::RateLimit(_) => "wait",
                Effect::Failed(_) => "fail",
            };
            assert_eq!(got, expected, "{provider}");
        }
    }
}
