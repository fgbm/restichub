//! The sign-in rate limit (ADR-0023, section Local authentication).
//!
//! The limiter is per process. A second control plane instance has its own buckets. This is
//! acceptable for the MVP, where one instance runs (ADR-0018); a shared limiter arrives with high
//! availability.
//!
//! A capped limiter holds the synthetic identities and the IP networks. It has a hard cap
//! ([`MAX_ENTRIES`]) and evicts the oldest buckets when the cap is reached, instead of growing. An
//! uncapped limiter holds the resolved `(tenant_id, user_id)` identities, so its size is bounded by
//! the number of real users.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use restichub_db::UserId;

pub const MAX_ENTRIES: usize = 100_000;

const SWEEP_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    last: Instant,
}

#[derive(Debug)]
struct State {
    buckets: HashMap<String, Bucket>,
    last_sweep: Instant,
}

#[derive(Debug)]
pub struct RateLimiter {
    capacity: f64,
    window: Duration,
    /// Zero means no cap.
    max_entries: usize,
    state: Mutex<State>,
}

impl RateLimiter {
    /// Creates a limiter with the given capacity for each bucket and the given refill window.
    ///
    /// A `max_entries` of zero creates an uncapped limiter. The cap of a capped limiter is at least
    /// one.
    pub fn new(capacity: u32, window: Duration, max_entries: usize) -> Self {
        Self {
            capacity: f64::from(capacity.max(1)),
            window,
            max_entries,
            state: Mutex::new(State {
                buckets: HashMap::new(),
                last_sweep: Instant::now(),
            }),
        }
    }

    /// The error is the time to wait on a refusal. A capped limiter evicts the oldest buckets when
    /// it is full.
    pub fn check(&self, key: &str, now: Instant) -> Result<(), Duration> {
        let refill_per_second = self.capacity / self.window.as_secs_f64().max(f64::EPSILON);
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        if now.saturating_duration_since(state.last_sweep) >= SWEEP_INTERVAL {
            sweep(&mut state.buckets, self.capacity, self.window, now);
            state.last_sweep = now;
        }
        if !state.buckets.contains_key(key) {
            if self.max_entries > 0 && state.buckets.len() >= self.max_entries {
                // Evict the oldest buckets rather than refuse a new key.
                let count = (self.max_entries / 100).max(1);
                evict_oldest(&mut state.buckets, count);
            }
            state.buckets.insert(
                key.to_owned(),
                Bucket {
                    tokens: self.capacity,
                    last: now,
                },
            );
        }
        let bucket = match state.buckets.get_mut(key) {
            Some(bucket) => bucket,
            None => return Err(self.window),
        };
        let elapsed = now.saturating_duration_since(bucket.last).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * refill_per_second).min(self.capacity);
        bucket.last = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            let missing = 1.0 - bucket.tokens;
            let wait = missing / refill_per_second;
            Err(Duration::from_secs_f64(wait.max(0.001)))
        }
    }

    pub fn sweep(&self, now: Instant) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        sweep(&mut state.buckets, self.capacity, self.window, now);
        state.last_sweep = now;
    }

    pub fn len(&self) -> usize {
        let state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.buckets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A full bucket holds no useful state. An idle bucket is full again after the window, so it holds
/// no useful state either.
fn sweep(buckets: &mut HashMap<String, Bucket>, capacity: f64, window: Duration, now: Instant) {
    buckets.retain(|_, bucket| {
        let idle = now.saturating_duration_since(bucket.last) >= window;
        bucket.tokens < capacity && !idle
    });
}

fn evict_oldest(buckets: &mut HashMap<String, Bucket>, count: usize) {
    if count == 0 || buckets.is_empty() {
        return;
    }
    if count >= buckets.len() {
        buckets.clear();
        return;
    }
    let mut entries: Vec<(Instant, String)> = buckets
        .iter()
        .map(|(key, bucket)| (bucket.last, key.clone()))
        .collect();
    entries.select_nth_unstable_by_key(count, |(last, _)| *last);
    for (_, key) in entries.into_iter().take(count) {
        buckets.remove(&key);
    }
}

/// An IPv6 address is keyed by the /64 network, so one host cannot rotate through the addresses of
/// one network to avoid the limit.
pub fn ip_key(ip: IpAddr) -> String {
    match ip.to_canonical() {
        IpAddr::V4(address) => format!("ip:{address}"),
        IpAddr::V6(address) => {
            let segments = address.segments();
            let network = Ipv6Addr::new(
                segments[0],
                segments[1],
                segments[2],
                segments[3],
                0,
                0,
                0,
                0,
            );
            format!("ip:{network}/64")
        }
    }
}

/// The username is trimmed and lowercased, so the case of the input does not create a new bucket.
/// The key is installation-wide: sign-in has no tenant (ADR-0025).
pub fn synthetic_user_key(username: &str) -> String {
    format!("s:{}", username.trim().to_lowercase())
}

pub fn resolved_user_key(user: UserId) -> String {
    format!("u:{user}")
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;
    use std::time::{Duration, Instant};

    use super::{RateLimiter, ip_key, resolved_user_key, synthetic_user_key};

    #[test]
    fn the_bucket_allows_the_capacity_then_refuses() {
        let limiter = RateLimiter::new(3, Duration::from_secs(60), 100);
        let now = Instant::now();
        for _ in 0..3 {
            assert!(limiter.check("u", now).is_ok());
        }
        let retry = limiter.check("u", now);
        assert!(retry.is_err(), "the fourth call must be refused");
        assert!(retry.err().unwrap_or_default() > Duration::ZERO);
    }

    #[test]
    fn the_bucket_refills_over_the_window() {
        let limiter = RateLimiter::new(2, Duration::from_secs(10), 100);
        let now = Instant::now();
        assert!(limiter.check("u", now).is_ok());
        assert!(limiter.check("u", now).is_ok());
        assert!(limiter.check("u", now).is_err());
        let later = now + Duration::from_secs(10);
        assert!(limiter.check("u", later).is_ok());
    }

    #[test]
    fn the_user_and_the_ip_buckets_are_separate() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60), 100);
        let now = Instant::now();
        assert!(limiter.check(&synthetic_user_key("u"), now).is_ok());
        assert!(limiter.check(&synthetic_user_key("u"), now).is_err());
        let ip: IpAddr = "203.0.113.7"
            .parse()
            .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
        assert!(limiter.check(&ip_key(ip), now).is_ok());
        assert!(limiter.check(&ip_key(ip), now).is_err());
        assert!(limiter.check(&synthetic_user_key("v"), now).is_ok());
    }

    #[test]
    fn the_synthetic_key_ignores_the_case_and_the_trim() {
        assert_eq!(synthetic_user_key(" Alice "), synthetic_user_key("alice"));
    }

    #[test]
    fn an_ipv6_address_is_keyed_by_the_slash_64_network() {
        let first: IpAddr = "2001:db8::1"
            .parse()
            .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
        let second: IpAddr = "2001:db8::2"
            .parse()
            .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
        let other: IpAddr = "2001:db8:0:1::1"
            .parse()
            .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
        assert_eq!(ip_key(first), ip_key(second));
        assert_ne!(ip_key(first), ip_key(other));
    }

    #[test]
    fn the_resolved_key_is_stable() {
        let user = restichub_db::UserId::from_uuid(uuid::Uuid::now_v7());
        assert_eq!(resolved_user_key(user), resolved_user_key(user));
    }

    #[test]
    fn the_capped_map_evicts_instead_of_growing() {
        // 10 000 distinct keys must not grow a capped map without a bound.
        let limiter = RateLimiter::new(1_000, Duration::from_secs(60), 128);
        let now = Instant::now();
        for index in 0..10_000 {
            let _ = limiter.check(&format!("s:{index}"), now);
        }
        assert!(limiter.len() <= 128, "the map holds {}", limiter.len());
        // A new key is still accepted after an eviction.
        assert!(limiter.check("s:new", now).is_ok());
    }

    #[test]
    fn an_uncapped_map_keeps_the_resolved_keys() {
        let limiter = RateLimiter::new(1_000, Duration::from_secs(60), 0);
        let now = Instant::now();
        for index in 0..10_000 {
            let _ = limiter.check(&format!("u:{index}"), now);
        }
        assert_eq!(limiter.len(), 10_000);
    }

    #[test]
    fn the_sweep_removes_the_idle_buckets() {
        let limiter = RateLimiter::new(5, Duration::from_secs(60), 100);
        let now = Instant::now();
        assert!(limiter.check("u", now).is_ok());
        assert_eq!(limiter.len(), 1);
        limiter.sweep(now + Duration::from_secs(61));
        assert_eq!(limiter.len(), 0);
    }
}
