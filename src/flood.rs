//! Sliding-window limits for incoming messages, joins, and CTCP replies.
//!
//! A limit of zero turns that check off. The caller supplies the clock so
//! tests can move time without sleeping.

use std::collections::{HashMap, HashSet};

pub const DEFAULT_MESSAGES: u32 = 10;
pub const DEFAULT_JOINS: u32 = 8;
pub const DEFAULT_WINDOW_MS: u64 = 5_000;
pub const DEFAULT_CTCP: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub messages: u32,
    pub joins: u32,
    pub window_ms: u64,
    /// CTCP requests answered per window, counted over everyone, so many
    /// senders together cannot make the server disconnect us for flooding.
    pub ctcp: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            messages: DEFAULT_MESSAGES,
            joins: DEFAULT_JOINS,
            window_ms: DEFAULT_WINDOW_MS,
            ctcp: DEFAULT_CTCP,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Holding,
    Tripped,
}

#[derive(Clone, Debug)]
pub struct Gate {
    limits: Limits,
    messages: Table,
    joins: Table,
    ctcp: Table,
}

/// Recent times for each sender (or channel), and who is being held back.
#[derive(Clone, Debug, Default)]
struct Table {
    buckets: HashMap<String, Vec<u64>>,
    holding: HashSet<String>,
    /// The size at which quiet entries are cleared out next. It grows with
    /// the number of entries still in use, so clearing out costs little on
    /// average however many senders are active.
    prune_at: usize,
    /// When entries were last cleared out. A large table is also cleared
    /// once a window has passed, so senders gone quiet do not stay until
    /// the table grows again.
    pruned_ms: u64,
}

/// Entries kept before the first clear-out.
const PRUNE_FROM: usize = 1_024;

impl Gate {
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            messages: Table::default(),
            joins: Table::default(),
            ctcp: Table::default(),
        }
    }

    pub fn set_limits(&mut self, limits: Limits) {
        self.limits = limits;
        self.messages = Table::default();
        self.joins = Table::default();
        self.ctcp = Table::default();
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    pub fn observe_message(&mut self, nick: &str, now_ms: u64) -> Verdict {
        let (limit, window) = (self.limits.messages, self.limits.window_ms);
        self.messages.observe(nick, now_ms, limit, window)
    }

    pub fn observe_join(&mut self, channel: &str, now_ms: u64) -> Verdict {
        let (limit, window) = (self.limits.joins, self.limits.window_ms);
        self.joins.observe(channel, now_ms, limit, window)
    }

    /// A CTCP request that wants an answer, from anyone.
    pub fn observe_ctcp(&mut self, now_ms: u64) -> Verdict {
        let (limit, window) = (self.limits.ctcp, self.limits.window_ms);
        self.ctcp.observe("*", now_ms, limit, window)
    }
}

impl Table {
    fn observe(&mut self, key: &str, now_ms: u64, limit: u32, window: u64) -> Verdict {
        if limit == 0 {
            return Verdict::Allow;
        }
        let name = key.to_ascii_lowercase();
        let large = self.buckets.len() >= PRUNE_FROM;
        if self.buckets.len() >= self.prune_at.max(PRUNE_FROM)
            || (large && now_ms.saturating_sub(self.pruned_ms) >= window)
        {
            self.prune(now_ms, window);
        }
        let stamps = self.buckets.entry(name.clone()).or_default();
        stamps.retain(|stamp| now_ms.saturating_sub(*stamp) < window);
        if stamps.len() as u32 >= limit {
            if self.holding.insert(name) {
                Verdict::Tripped
            } else {
                Verdict::Holding
            }
        } else {
            self.holding.remove(&name);
            stamps.push(now_ms);
            Verdict::Allow
        }
    }

    /// Forget everyone quiet for a whole window, held back or not: a sender
    /// being held has no time inside the window either, and its next
    /// message would be let through anyway.
    fn prune(&mut self, now_ms: u64, window: u64) {
        self.buckets.retain(|_, stamps| {
            stamps
                .last()
                .is_some_and(|last| now_ms.saturating_sub(*last) < window)
        });
        let buckets = &self.buckets;
        self.holding.retain(|name| buckets.contains_key(name));
        self.prune_at = (self.buckets.len() * 2).max(PRUNE_FROM);
        self.pruned_ms = now_ms;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_senders_are_forgotten() {
        let mut table = Table::default();
        for n in 0..5_000u64 {
            table.observe(&format!("nick{n}"), n * 10, 5, 1_000);
        }
        assert!(
            table.buckets.len() <= 2 * PRUNE_FROM,
            "{}",
            table.buckets.len()
        );
    }

    #[test]
    fn held_back_senders_are_forgotten_once_quiet() {
        let mut table = Table::default();
        // 1,500 senders each trip the limit, then never write again.
        for n in 0..1_500u64 {
            let nick = format!("flooder{n}");
            assert_eq!(table.observe(&nick, n, 1, 10_000), Verdict::Allow);
            assert_eq!(table.observe(&nick, n, 1, 10_000), Verdict::Tripped);
        }
        assert_eq!(table.holding.len(), 1_500);
        // Long after, new senders come along.
        for n in 0..100u64 {
            table.observe(&format!("new{n}"), 1_000_000 + n, 1, 10_000);
        }
        assert!(
            table.holding.is_empty(),
            "{} still held",
            table.holding.len()
        );
        assert!(table.buckets.len() <= 100, "{} kept", table.buckets.len());
        // The clear-out does not run again on every message.
        assert!(table.prune_at >= PRUNE_FROM);
    }

    #[test]
    fn clearing_out_costs_little_with_many_active_senders() {
        // 3,000 senders all active inside the window: the table is scanned
        // only when it has doubled, not on every message.
        let mut table = Table::default();
        let mut scans = 0;
        for n in 0..3_000u64 {
            let before = (table.prune_at, table.pruned_ms);
            table.observe(&format!("busy{n}"), n, 5, 1_000_000);
            if (table.prune_at, table.pruned_ms) != before {
                scans += 1;
            }
        }
        assert!(scans <= 3, "{scans} scans");
        assert_eq!(table.buckets.len(), 3_000, "active senders are all kept");
    }

    fn tight() -> Gate {
        Gate::new(Limits {
            messages: 2,
            joins: 2,
            window_ms: 1_000,
            ctcp: 2,
        })
    }

    #[test]
    fn third_message_trips_and_the_next_is_held() {
        let mut gate = tight();
        assert_eq!(gate.observe_message("Alice", 0), Verdict::Allow);
        assert_eq!(gate.observe_message("alice", 10), Verdict::Allow);
        assert_eq!(gate.observe_message("ALICE", 20), Verdict::Tripped);
        assert_eq!(gate.observe_message("alice", 30), Verdict::Holding);
    }

    #[test]
    fn nicks_and_channels_are_counted_apart() {
        let mut gate = tight();
        assert_eq!(gate.observe_message("alice", 0), Verdict::Allow);
        assert_eq!(gate.observe_message("bob", 0), Verdict::Allow);
        assert_eq!(gate.observe_join("#openirc", 0), Verdict::Allow);
        assert_eq!(gate.observe_join("#rust", 0), Verdict::Allow);
        assert_eq!(gate.observe_message("alice", 1), Verdict::Allow);
        assert_eq!(gate.observe_join("#openirc", 1), Verdict::Allow);
    }

    #[test]
    fn the_window_lets_a_nick_through_again() {
        let mut gate = tight();
        assert_eq!(gate.observe_message("alice", 0), Verdict::Allow);
        assert_eq!(gate.observe_message("alice", 10), Verdict::Allow);
        assert_eq!(gate.observe_message("alice", 20), Verdict::Tripped);
        assert_eq!(gate.observe_message("alice", 1_010), Verdict::Allow);
    }

    #[test]
    fn a_zero_limit_never_trips() {
        let mut gate = Gate::new(Limits {
            messages: 0,
            joins: 0,
            window_ms: 1_000,
            ctcp: 0,
        });
        for now in 0..5 {
            assert_eq!(gate.observe_message("alice", now), Verdict::Allow);
            assert_eq!(gate.observe_join("#openirc", now), Verdict::Allow);
            assert_eq!(gate.observe_ctcp(now), Verdict::Allow);
        }
    }

    #[test]
    fn ctcp_answers_are_counted_over_everyone() {
        let mut gate = tight();
        assert_eq!(gate.observe_ctcp(0), Verdict::Allow);
        assert_eq!(gate.observe_ctcp(1), Verdict::Allow);
        assert_eq!(gate.observe_ctcp(2), Verdict::Tripped);
        assert_eq!(gate.observe_ctcp(3), Verdict::Holding);
        assert_eq!(gate.observe_ctcp(1_001), Verdict::Allow);
    }
}
