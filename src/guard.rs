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

#[derive(Debug)]
pub struct RateLimit {
    window: Duration,
    windows: Mutex<HashMap<Class, Window>>,
}

impl RateLimit {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            windows: Mutex::new(HashMap::new()),
        }
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
                if entry.calls < class.budget() {
                    entry.calls += 1;
                    None
                } else {
                    Some(self.window - now.duration_since(entry.started))
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

#[derive(Debug, Clone)]
pub struct BreakerConfig {
    pub failure_threshold: u32,
    pub failure_window: Duration,
    pub open_for: Duration,
}

impl Default for BreakerConfig {
    fn default() -> Self {
        Self {
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
    config: BreakerConfig,
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
            config,
            failures: Mutex::new(Vec::new()),
            state: Mutex::new(BreakerState::Closed),
        }
    }

    /// Why a call was refused, or `None` when it may proceed.
    pub fn check(&self) -> Option<String> {
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
        let now = Instant::now();
        let mut failures = self.failures.lock().expect("breaker");
        failures.retain(|t| now.duration_since(*t) <= self.config.failure_window);
        failures.push(now);
        if failures.len() as u32 >= self.config.failure_threshold {
            failures.clear();
            *self.state.lock().expect("breaker") = BreakerState::Open {
                until: now + self.config.open_for,
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
    fn the_breaker_opens_after_the_threshold_and_half_opens_after_the_wait() {
        let breaker = CircuitBreaker::new(BreakerConfig {
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
