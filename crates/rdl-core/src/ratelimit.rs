//! Speed limits: token buckets that bytes have to pass through.
//!
//! The engine takes every received chunk from each bucket that applies to
//! the download (the global one and the download's own). A bucket refills at
//! its rate and holds at most a quarter-second's worth, so a limit is felt
//! within a moment of being set. Taking more than is there puts the bucket
//! in debt and the taker sleeps until it's paid off; with several
//! connections sharing a bucket, each waits its turn and the total stays at
//! the rate. Rates can change at any time; `None` means unlimited.

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::time::Instant;

/// Slowest rate a limit can be set to, so a typo can't stall everything.
pub const MIN_RATE: u64 = 1024;

pub struct RateLimiter {
    state: Mutex<State>,
}

struct State {
    rate: Option<u64>,
    /// Bytes that may pass now; negative is debt.
    tokens: f64,
    last: Instant,
}

/// The most a bucket holds: a quarter second at its rate (at least 16 KiB,
/// one network chunk).
fn burst(rate: u64) -> f64 {
    (rate as f64 / 4.0).max(16.0 * 1024.0)
}

impl RateLimiter {
    pub fn new(rate: Option<u64>) -> Self {
        let rate = rate.map(|r| r.max(MIN_RATE));
        Self {
            state: Mutex::new(State {
                rate,
                tokens: rate.map(burst).unwrap_or(0.0),
                last: Instant::now(),
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn rate(&self) -> Option<u64> {
        self.lock().rate
    }

    /// Change the rate; takers already waiting finish their current wait.
    pub fn set_rate(&self, rate: Option<u64>) {
        let rate = rate.map(|r| r.max(MIN_RATE));
        let mut state = self.lock();
        if state.rate == rate {
            return;
        }
        state.rate = rate;
        state.last = Instant::now();
        // Start the new rate fresh: no debt from a slower one, no hoard from a faster one.
        state.tokens = rate.map(burst).unwrap_or(0.0);
    }

    /// How long to wait before `n` more bytes may pass, booking them now.
    fn reserve(&self, n: u64) -> Option<Duration> {
        let mut state = self.lock();
        let rate = state.rate?;
        let now = Instant::now();
        let elapsed = now.duration_since(state.last).as_secs_f64();
        state.last = now;
        state.tokens = (state.tokens + elapsed * rate as f64).min(burst(rate));
        state.tokens -= n as f64;
        (state.tokens < 0.0).then(|| Duration::from_secs_f64(-state.tokens / rate as f64))
    }

    /// Wait until `n` bytes may pass. Cancel by dropping the future.
    pub async fn take(&self, n: u64) {
        if let Some(wait) = self.reserve(n) {
            tokio::time::sleep(wait).await;
        }
    }
}

impl fmt::Debug for RateLimiter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RateLimiter")
            .field("rate", &self.rate())
            .finish()
    }
}

/// The buckets one download's bytes pass through, in order.
#[derive(Debug, Clone, Default)]
pub struct Throttle(Vec<Arc<RateLimiter>>);

impl Throttle {
    pub fn new(limiters: impl IntoIterator<Item = Arc<RateLimiter>>) -> Self {
        Self(limiters.into_iter().collect())
    }

    /// Wait until `n` bytes may pass every bucket.
    pub async fn take(&self, n: u64) {
        for limiter in &self.0 {
            limiter.take(n).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bytes per second actually let through when `takers` tasks push
    /// `chunk`-sized pieces for `secs` of paused (virtual) time.
    async fn measure(limiter: Arc<RateLimiter>, takers: usize, chunk: u64, secs: u64) -> f64 {
        let start = Instant::now();
        let deadline = start + Duration::from_secs(secs);
        let mut tasks = Vec::new();
        for _ in 0..takers {
            let limiter = Arc::clone(&limiter);
            tasks.push(tokio::spawn(async move {
                let mut passed = 0u64;
                while Instant::now() < deadline {
                    limiter.take(chunk).await;
                    passed += chunk;
                }
                passed
            }));
        }
        let mut total = 0;
        for t in tasks {
            total += t.await.unwrap();
        }
        total as f64 / start.elapsed().as_secs_f64()
    }

    #[tokio::test(start_paused = true)]
    async fn holds_the_rate_for_one_or_many_takers() {
        for takers in [1, 8] {
            let rate = 1024 * 1024;
            let got = measure(
                Arc::new(RateLimiter::new(Some(rate))),
                takers,
                16 * 1024,
                20,
            )
            .await;
            let ratio = got / rate as f64;
            assert!((0.95..1.1).contains(&ratio), "{takers} takers: {got} B/s");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn chunks_bigger_than_the_bucket_still_pass_at_the_rate() {
        let rate = 64 * 1024;
        let got = measure(Arc::new(RateLimiter::new(Some(rate))), 2, 256 * 1024, 30).await;
        let ratio = got / rate as f64;
        assert!((0.9..1.2).contains(&ratio), "{got} B/s");
    }

    #[tokio::test(start_paused = true)]
    async fn unlimited_and_changed_rates() {
        let limiter = Arc::new(RateLimiter::new(None));
        let start = Instant::now();
        limiter.take(u64::MAX / 2).await;
        assert_eq!(start.elapsed(), Duration::ZERO, "unlimited never waits");

        limiter.set_rate(Some(100 * 1024));
        let slow = measure(Arc::clone(&limiter), 1, 8 * 1024, 10).await;
        limiter.set_rate(Some(400 * 1024));
        let fast = measure(Arc::clone(&limiter), 1, 8 * 1024, 10).await;
        assert!((3.5..4.5).contains(&(fast / slow)), "{slow} then {fast}");

        limiter.set_rate(Some(1));
        assert_eq!(
            limiter.rate(),
            Some(MIN_RATE),
            "absurdly low rates are raised"
        );
        limiter.set_rate(None);
        assert_eq!(limiter.rate(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn a_throttle_obeys_its_slowest_bucket() {
        let global = Arc::new(RateLimiter::new(Some(1024 * 1024)));
        let own = Arc::new(RateLimiter::new(Some(128 * 1024)));
        let throttle = Throttle::new([global, own]);
        let start = Instant::now();
        let mut passed = 0u64;
        while start.elapsed() < Duration::from_secs(10) {
            throttle.take(16 * 1024).await;
            passed += 16 * 1024;
        }
        let rate = passed as f64 / start.elapsed().as_secs_f64();
        assert!(
            (120.0 * 1024.0..140.0 * 1024.0).contains(&rate),
            "{rate} B/s"
        );
    }
}
