//! Counts failures (wrong passwords) per IP and per key in fixed windows, in memory: the process is single.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::Mutex;

pub struct FailureLimiter {
    per_ip: u32,
    per_key: u32,
    window: i64,
    max_entries: usize,
    state: Mutex<Windows>,
}

/// `order` lists windows oldest first; an entry whose start no longer matches the map is stale and skipped.
#[derive(Default)]
struct Windows {
    map: HashMap<String, (i64, u32)>,
    order: VecDeque<(String, i64)>,
}

impl FailureLimiter {
    /// `per_ip` 0 turns the IP side off. Callers choose IPs and keys, so the maps are capped at `max_entries`.
    pub fn new(per_ip: u32, per_key: u32, window_secs: i64, max_entries: usize) -> Self {
        FailureLimiter {
            per_ip,
            per_key,
            window: window_secs,
            max_entries,
            state: Mutex::default(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Windows> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Seconds to wait, or `None` to go ahead.
    pub fn blocked(&self, ip: IpAddr, key: &str, now: i64) -> Option<i64> {
        let w = self.lock();
        let wait = |id: String, limit: u32| -> i64 {
            match w.map.get(&id) {
                Some(&(start, count))
                    if limit > 0 && count >= limit && now - start < self.window =>
                {
                    start + self.window - now
                }
                _ => 0,
            }
        };
        let longest =
            wait(format!("ip:{ip}"), self.per_ip).max(wait(format!("key:{key}"), self.per_key));
        (longest > 0).then_some(longest)
    }

    pub fn failed(&self, ip: IpAddr, key: &str, now: i64) {
        let mut w = self.lock();
        while let Some((id, start)) = w.order.front().cloned() {
            let current = w.map.get(&id).map(|e| e.0);
            if current == Some(start) && now - start < self.window {
                break;
            }
            w.order.pop_front();
            if current == Some(start) {
                w.map.remove(&id);
            }
        }
        let mut ids = vec![format!("key:{key}")];
        if self.per_ip > 0 {
            ids.push(format!("ip:{ip}"));
        }
        for id in ids {
            match w.map.get_mut(&id) {
                Some(entry) if now - entry.0 < self.window => entry.1 += 1,
                _ => {
                    w.map.insert(id.clone(), (now, 1));
                    w.order.push_back((id, now));
                }
            }
        }
        while w.map.len() > self.max_entries {
            let Some((id, start)) = w.order.pop_front() else {
                break;
            };
            if w.map.get(&id).map(|e| e.0) == Some(start) {
                w.map.remove(&id);
            }
        }
    }

    pub fn succeeded(&self, key: &str) {
        self.lock().map.remove(&format!("key:{key}"));
    }

    pub fn len(&self) -> usize {
        self.lock().map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The sign-in limits from the spec: fixed, with no switch to turn them off.
pub struct Limiters {
    /// Failed sign-ins: 10 per IP and 5 per e-mail in 15 minutes.
    pub login: FailureLimiter,
    /// Wrong current password on "Change password": 5 per user in 15 minutes.
    pub password_change: FailureLimiter,
}

impl Default for Limiters {
    fn default() -> Self {
        Self::new()
    }
}

impl Limiters {
    pub fn new() -> Self {
        Limiters {
            login: FailureLimiter::new(10, 5, 15 * 60, 10_000),
            password_change: FailureLimiter::new(0, 5, 15 * 60, 10_000),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(n: u8) -> IpAddr {
        IpAddr::from([203, 0, 113, n])
    }

    #[test]
    fn blocks_after_five_for_one_email() {
        let l = FailureLimiter::new(10, 5, 900, 10_000);
        for i in 0..5 {
            assert_eq!(l.blocked(ip(i), "ana@example.com", 0), None);
            l.failed(ip(i), "ana@example.com", 0);
        }
        assert_eq!(l.blocked(ip(9), "ana@example.com", 0), Some(900));
        assert_eq!(l.blocked(ip(9), "bo@example.com", 0), None);
    }

    #[test]
    fn blocks_after_ten_for_one_ip() {
        let l = FailureLimiter::new(10, 5, 900, 10_000);
        for i in 0..10 {
            l.failed(ip(1), &format!("u{i}@example.com"), 0);
        }
        assert_eq!(l.blocked(ip(1), "new@example.com", 60), Some(840));
        assert_eq!(l.blocked(ip(2), "new@example.com", 60), None);
    }

    #[test]
    fn window_resets() {
        let l = FailureLimiter::new(10, 5, 900, 10_000);
        for _ in 0..5 {
            l.failed(ip(1), "ana@example.com", 0);
        }
        assert_eq!(l.blocked(ip(1), "ana@example.com", 900), None);
    }

    #[test]
    fn success_clears_key() {
        let l = FailureLimiter::new(10, 5, 900, 10_000);
        for _ in 0..5 {
            l.failed(ip(1), "ana@example.com", 0);
        }
        l.succeeded("ana@example.com");
        assert_eq!(l.blocked(ip(2), "ana@example.com", 0), None);
    }

    #[test]
    fn bounded() {
        let l = FailureLimiter::new(10, 5, 900, 100);
        for i in 0..1000 {
            l.failed(ip((i % 250) as u8), &format!("u{i}@example.com"), 0);
        }
        assert!(l.len() <= 100, "{}", l.len());
    }

    #[test]
    fn ip_side_disabled() {
        let l = FailureLimiter::new(0, 5, 900, 10_000);
        for i in 0..20 {
            l.failed(ip(1), &format!("user:{i}"), 0);
        }
        assert_eq!(l.blocked(ip(1), "user:99", 0), None);
    }
}
