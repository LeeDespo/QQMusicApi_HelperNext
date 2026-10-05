//! Politeness: a rate limiter and a circuit breaker, both on the request path.
//!
//! Why they exist here rather than in the app: this component is the only thing
//! that talks to the upstream now, so it is the only place that can see *all* of
//! the traffic. The upstream rate-limits aggressively (the app already had to
//! learn about `RatelimitedError`), and a burst of page loads must not become a
//! burst of upstream requests.
//!
//! The shape follows what the app's settings already expose for the old helper's
//! breaker, so the settings keep meaning the same thing:
//!
//! * **rate limit** — a sliding window per endpoint class: at most `max_calls`
//!   in `window`. Calls over the limit wait (they are not dropped: every caller
//!   here is a user-visible read, and failing it would be worse than delaying it).
//! * **circuit breaker** — after `failure_threshold` failures inside
//!   `failure_window` the circuit opens for `open_seconds`, during which calls
//!   fail immediately (with the reason) instead of hammering a broken upstream;
//!   then one probe is let through to test the water.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Coarse classes: the upstream counts per endpoint family, and so do we.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class {
    /// Catalogue reads that a page load needs.
    Read,
    /// Search and radio: the user can trigger these as fast as they can type.
    Interactive,
    /// Playback tickets and lyric fetches, which ride along with a download.
    Playback,
    /// Account reads (我喜欢, playlists) — the ones most likely to be rate limited.
    Account,
    /// Writes. Deliberately the tightest budget: a like is one click, and a
    /// runaway loop here would be visible to the account's owner.
    Write,
}

impl Class {
    /// Requests allowed inside `RateLimit::window`.
    fn budget(self) -> u32 {
        match self {
            Class::Read => 30,
            Class::Interactive => 12,
            Class::Playback => 20,
            Class::Account => 12,
            Class::Write => 6,
        }
    }
}

#[derive(Debug)]
struct Window {
    started: Instant,
    calls: u32,
}

/// The ceiling the user sets from the app's settings.
///
/// A single window across *every* class, on top of the per-class budgets: the
/// per-class numbers are what keeps a page load from becoming a burst, and this
/// is the blunt "no more than N requests per M seconds, whatever they are" that
/// answers "am I about to be rate limited by the upstream". Off by default means
/// the per-class budgets alone, i.e. what the component has always done.
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    pub enabled: bool,
    pub window: Duration,
    pub max_calls: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            window: Duration::from_secs(10),
            max_calls: 100,
        }
    }
}

#[derive(Debug)]
struct GlobalWindow {
    config: RateLimitConfig,
    started: Instant,
    calls: u32,
}

#[derive(Debug)]
pub struct RateLimit {
    window: Duration,
    windows: Mutex<HashMap<Class, Window>>,
    global: Mutex<GlobalWindow>,
}

impl RateLimit {
    pub fn new(window: Duration) -> Self {
        let config = RateLimitConfig::default();
        Self {
            window,
            windows: Mutex::new(HashMap::new()),
            global: Mutex::new(GlobalWindow {
                config,
                started: Instant::now(),
                calls: 0,
            }),
        }
    }

    /// Apply the user's ceiling. Called when the setting changes and on startup.
    pub fn configure(&self, config: RateLimitConfig) {
        let mut global = self.global.lock().expect("rate limit");
        global.config = config;
        // A new window starts here rather than carrying the old count over: the
        // number the user just typed must take effect now, not after the old
        // window happens to expire.
        global.started = Instant::now();
        global.calls = 0;
    }

    pub fn config(&self) -> RateLimitConfig {
        self.global.lock().expect("rate limit").config.clone()
    }

    /// Blocks until this class may make a call, then records it.
    ///
    /// Sleeping while holding the lock would serialize every caller behind the
    /// sleeper, so the wait is computed under the lock and taken outside it, then
    /// re-checked — the same optimistic pattern as a spin lock, with a sleep.
    pub fn acquire(&self, class: Class) {
        loop {
            let wait = {
                let mut windows = self.windows.lock().expect("rate limit");
                let now = Instant::now();
                let entry = windows.entry(class).or_insert(Window {
                    started: now,
                    calls: 0,
                });
                if now.duration_since(entry.started) >= self.window {
                    *entry = Window {
                        started: now,
                        calls: 0,
                    };
                }
                // The class budget is checked first because it is the tighter
                // one in practice; the user's global ceiling then applies to
                // everything that got this far. Both are *waits*, never drops:
                // every caller here is a read someone is waiting for, and
                // delaying it beats failing it — the upstream's own limit answers
                // with an empty result, which is the outcome we are avoiding.
                let class_wait = if entry.calls < class.budget() {
                    None
                } else {
                    Some(self.window - now.duration_since(entry.started))
                };
                let mut global = self.global.lock().expect("rate limit");
                let mut global_wait = None;
                if global.config.enabled {
                    if now.duration_since(global.started) >= global.config.window {
                        global.started = now;
                        global.calls = 0;
                    }
                    if global.calls >= global.config.max_calls {
                        global_wait =
                            Some(global.config.window - now.duration_since(global.started));
                    }
                }
                match (class_wait, global_wait) {
                    (None, None) => {
                        entry.calls += 1;
                        if global.config.enabled {
                            global.calls += 1;
                        }
                        None
                    }
                    // Both windows can be closed at once; wait for whichever
                    // opens last.
                    (class_wait, global_wait) => Some(
                        class_wait
                            .unwrap_or_default()
                            .max(global_wait.unwrap_or_default()),
                    ),
                }
            };
            match wait {
                None => return,
                Some(duration) if duration.is_zero() => continue,
                Some(duration) => std::thread::sleep(duration.min(Duration::from_secs(2))),
            }
        }
    }

    /// How many calls this class has made inside the current window. Exposed for
    /// the status report, so "am I being throttled by my own limiter?" is
    /// answerable from outside.
    pub fn usage(&self, class: Class) -> u32 {
        self.windows
            .lock()
            .expect("rate limit")
            .get(&class)
            .map(|window| window.calls)
            .unwrap_or(0)
    }
}

/// When the breaker opens, and whether it is allowed to.
///
/// Adjustable from the app (`set_breaker`), which is why every field lives
/// together here rather than being baked into the breaker: the numbers the user
/// sees in 设置 → Helper 组件 are the ones this enforces.
#[derive(Debug, Clone)]
pub struct BreakerConfig {
    /// Off means failures are still *reported* to the caller but never open the
    /// circuit — the app's own mirror keeps its own count either way.
    pub enabled: bool,
    pub failure_threshold: u32,
    pub failure_window: Duration,
    pub open_for: Duration,
}

impl Default for BreakerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            failure_threshold: 5,
            failure_window: Duration::from_secs(60),
            open_for: Duration::from_secs(30),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BreakerState {
    Closed,
    Open { until: Instant },
    HalfOpen,
}

#[derive(Debug)]
pub struct CircuitBreaker {
    /// Behind a lock because the app can change it at any time; read on every
    /// request, so it must not be copied into each call site.
    config: Mutex<BreakerConfig>,
    failures: Mutex<Vec<Instant>>,
    state: Mutex<BreakerState>,
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new(BreakerConfig::default())
    }
}

impl CircuitBreaker {
    pub fn new(config: BreakerConfig) -> Self {
        Self {
            config: Mutex::new(config),
            failures: Mutex::new(Vec::new()),
            state: Mutex::new(BreakerState::Closed),
        }
    }

    /// Apply the user's numbers. Called when a setting changes.
    ///
    /// The state is cleared as well as the configuration replaced: tuning the
    /// breaker is a deliberate act, and leaving it open (or half-open) would mean
    /// the new numbers do not take effect until the old open period expires —
    /// which reads as "the setting did nothing".
    pub fn configure(&self, config: BreakerConfig) {
        *self.config.lock().expect("breaker") = config;
        self.failures.lock().expect("breaker").clear();
        *self.state.lock().expect("breaker") = BreakerState::Closed;
    }

    pub fn config(&self) -> BreakerConfig {
        self.config.lock().expect("breaker").clone()
    }

    /// Why a call was refused, or `None` when it may proceed.
    pub fn check(&self) -> Option<String> {
        if !self.config.lock().expect("breaker").enabled {
            return None;
        }
        let mut state = self.state.lock().expect("breaker");
        match *state {
            BreakerState::Closed => None,
            BreakerState::Open { until } => {
                if Instant::now() >= until {
                    *state = BreakerState::HalfOpen;
                    None
                } else {
                    let remaining = until.duration_since(Instant::now()).as_secs();
                    Some(format!("上游连续失败，已熔断，约 {remaining} 秒后重试"))
                }
            }
            // A half-open circuit lets exactly one probe through; further calls
            // wait, so a broken upstream is not hammered by a burst.
            BreakerState::HalfOpen => Some("正在探测上游是否恢复，请稍后再试".into()),
        }
    }

    pub fn record_success(&self) {
        self.failures.lock().expect("breaker").clear();
        *self.state.lock().expect("breaker") = BreakerState::Closed;
    }

    pub fn record_failure(&self) {
        let config = self.config();
        if !config.enabled {
            return;
        }
        let now = Instant::now();
        let mut failures = self.failures.lock().expect("breaker");
        failures.retain(|t| now.duration_since(*t) <= config.failure_window);
        failures.push(now);
        if failures.len() as u32 >= config.failure_threshold {
            failures.clear();
            *self.state.lock().expect("breaker") = BreakerState::Open {
                until: now + config.open_for,
            };
        }
    }

    pub fn state(&self) -> BreakerState {
        self.state.lock().expect("breaker").clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rate_limiter_blocks_only_over_budget() {
        let limiter = RateLimit::new(Duration::from_millis(120));
        // `Write` has the smallest budget, so it is the cheap one to exhaust.
        for _ in 0..Class::Write.budget() {
            limiter.acquire(Class::Write);
        }
        assert_eq!(limiter.usage(Class::Write), Class::Write.budget());
        let started = Instant::now();
        limiter.acquire(Class::Write); // waits for the window to roll over
        assert!(started.elapsed() >= Duration::from_millis(100));
        assert_eq!(limiter.usage(Class::Write), 1, "window restarted");
    }

    #[test]
    fn the_global_ceiling_waits_and_then_reopens() {
        let limiter = RateLimit::new(Duration::from_millis(50));
        limiter.configure(RateLimitConfig {
            enabled: true,
            window: Duration::from_millis(120),
            max_calls: 2,
        });
        let started = Instant::now();
        for _ in 0..4 {
            limiter.acquire(Class::Read);
        }
        // Four calls through a two-per-window ceiling cannot finish faster than
        // one window: the third waits for the next window to open.
        assert!(
            started.elapsed() >= Duration::from_millis(100),
            "the ceiling did not delay anything: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_disabled_ceiling_leaves_the_class_budgets_alone() {
        let limiter = RateLimit::new(Duration::from_millis(50));
        limiter.configure(RateLimitConfig {
            enabled: false,
            window: Duration::from_secs(60),
            max_calls: 1,
        });
        let started = Instant::now();
        // One class budget's worth of writes, which the disabled ceiling must not
        // touch (it would have let exactly one through and stalled on the rest).
        for _ in 0..Class::Write.budget() {
            limiter.acquire(Class::Write);
        }
        assert!(
            started.elapsed() < Duration::from_millis(40),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn the_threshold_can_be_changed_while_the_breaker_is_in_use() {
        let breaker = CircuitBreaker::default();
        breaker.configure(BreakerConfig {
            enabled: true,
            failure_threshold: 2,
            failure_window: Duration::from_secs(60),
            open_for: Duration::from_secs(30),
        });
        breaker.record_failure();
        assert!(
            breaker.check().is_none(),
            "one failure is under a threshold of two"
        );
        breaker.record_failure();
        assert!(breaker.check().is_some(), "the second must open it");
    }

    #[test]
    fn a_disabled_breaker_stops_opening_and_clears_what_it_had() {
        let breaker = CircuitBreaker::default();
        breaker.record_failure();
        breaker.record_failure();
        breaker.record_failure();
        breaker.record_failure();
        breaker.record_failure();
        assert!(
            breaker.check().is_some(),
            "five failures open the default breaker"
        );
        breaker.configure(BreakerConfig {
            enabled: false,
            ..BreakerConfig::default()
        });
        assert!(
            breaker.check().is_none(),
            "disabling must take effect at once"
        );
        for _ in 0..20 {
            breaker.record_failure();
        }
        assert!(
            breaker.check().is_none(),
            "and no later failure may reopen it"
        );
    }

    #[test]
    fn the_breaker_opens_after_the_threshold_and_half_opens_after_the_wait() {
        let breaker = CircuitBreaker::new(BreakerConfig {
            enabled: true,
            failure_threshold: 3,
            failure_window: Duration::from_secs(5),
            open_for: Duration::from_millis(80),
        });
        assert!(breaker.check().is_none());
        for _ in 0..3 {
            breaker.record_failure();
        }
        assert!(breaker.check().is_some(), "opened");
        std::thread::sleep(Duration::from_millis(100));
        assert!(breaker.check().is_none(), "one probe is let through");
        assert!(breaker.check().is_some(), "the rest wait for the probe");
        breaker.record_success();
        assert!(breaker.check().is_none(), "closed again");
    }
}
