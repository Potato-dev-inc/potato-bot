//! A tiny sliding-window rate limiter, keyed by a string. Time is passed in, so tests need no clock.

use std::collections::{HashMap, VecDeque};

use parking_lot::Mutex;

#[derive(Default)]
pub struct Limiter {
    hits: Mutex<HashMap<String, VecDeque<u64>>>,
}

impl Limiter {
    /// Counts one hit for `key` and says whether it is allowed: at most `limit` hits in the last `window_ms`.
    /// A refused hit is not counted, so a flood does not keep the window shut for longer than the window.
    pub fn allow(&self, key: &str, limit: usize, window_ms: u64, now_ms: u64) -> bool {
        let mut hits = self.hits.lock();
        if hits.len() > 4096 {
            hits.retain(|_, q| q.back().is_some_and(|t| now_ms.saturating_sub(*t) < window_ms));
        }
        let queue = hits.entry(key.to_string()).or_default();
        while queue.front().is_some_and(|t| now_ms.saturating_sub(*t) >= window_ms) {
            queue.pop_front();
        }
        if queue.len() >= limit {
            return false;
        }
        queue.push_back(now_ms);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_the_limit_then_refuses_until_the_window_moves() {
        let l = Limiter::default();
        for i in 0..3 {
            assert!(l.allow("k", 3, 1000, i), "hit {i}");
        }
        assert!(!l.allow("k", 3, 1000, 500));
        assert!(!l.allow("k", 3, 1000, 999));
        assert!(l.allow("k", 3, 1000, 1001), "the first hit left the window");
        assert!(l.allow("other", 3, 1000, 5), "keys are separate");
    }
}
