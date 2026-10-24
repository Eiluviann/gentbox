//! Rate limits (architecture §8): token buckets per `source.agent` (burst 30,
//! 1/s) and per conversation (burst 10, 1/s) on `POST /v1/messages`, and one
//! shared bucket for the unauthenticated `hello`. The body limit is 128 KiB.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

/// The limits a server enforces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    /// Largest request body, in bytes.
    pub body_bytes: usize,
    /// Messages one agent may send at once.
    pub agent_burst: f64,
    /// Messages per second an agent's bucket refills.
    pub agent_refill_per_second: f64,
    /// Messages one conversation may send at once.
    pub conversation_burst: f64,
    /// Messages per second a conversation's bucket refills.
    pub conversation_refill_per_second: f64,
    /// `hello` calls at once, across all clients.
    pub hello_burst: f64,
    /// `hello` calls per second the shared bucket refills.
    pub hello_refill_per_second: f64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            body_bytes: 128 * 1024,
            agent_burst: 30.0,
            agent_refill_per_second: 1.0,
            conversation_burst: 10.0,
            conversation_refill_per_second: 1.0,
            hello_burst: 30.0,
            hello_refill_per_second: 5.0,
        }
    }
}

/// How many buckets are kept before full (idle) ones are dropped.
const MAX_BUCKETS: usize = 4096;

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    updated: Instant,
}

impl Bucket {
    fn refilled(self, now: Instant, burst: f64, per_second: f64) -> Self {
        let elapsed = now.saturating_duration_since(self.updated).as_secs_f64();
        Self {
            tokens: (self.tokens + elapsed * per_second).min(burst),
            updated: now,
        }
    }

    /// Seconds until one token is back.
    fn wait_seconds(self, per_second: f64) -> u64 {
        let missing = (1.0 - self.tokens).max(0.0);
        // At least one second: `Retry-After` counts whole seconds.
        let seconds = (missing / per_second).ceil();
        if seconds.is_finite() && seconds >= 1.0 {
            // A refill rate of at least 1/3600 s keeps this small.
            seconds.min(3600.0) as u64
        } else {
            1
        }
    }
}

#[derive(Debug, Default)]
struct Buckets {
    agents: HashMap<String, Bucket>,
    conversations: HashMap<(String, String), Bucket>,
    hello: Option<Bucket>,
}

/// The token buckets of one server.
#[derive(Debug)]
pub struct RateLimiter {
    limits: Limits,
    buckets: Mutex<Buckets>,
}

impl RateLimiter {
    /// A limiter with full buckets.
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            buckets: Mutex::new(Buckets::default()),
        }
    }

    /// The limits it enforces.
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Takes one token from the agent's and the conversation's buckets, or
    /// neither: `Err` carries the `Retry-After` seconds.
    pub fn check_message(&self, agent: &str, conversation_id: &str) -> Result<(), u64> {
        let now = Instant::now();
        let limits = self.limits;
        let mut buckets = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
        let full = |burst| Bucket {
            tokens: burst,
            updated: now,
        };
        let agent_bucket = buckets
            .agents
            .get(agent)
            .copied()
            .unwrap_or(full(limits.agent_burst))
            .refilled(now, limits.agent_burst, limits.agent_refill_per_second);
        let conversation_key = (agent.to_owned(), conversation_id.to_owned());
        let conversation_bucket = buckets
            .conversations
            .get(&conversation_key)
            .copied()
            .unwrap_or(full(limits.conversation_burst))
            .refilled(
                now,
                limits.conversation_burst,
                limits.conversation_refill_per_second,
            );
        if agent_bucket.tokens < 1.0 {
            return Err(agent_bucket.wait_seconds(limits.agent_refill_per_second));
        }
        if conversation_bucket.tokens < 1.0 {
            return Err(conversation_bucket.wait_seconds(limits.conversation_refill_per_second));
        }
        if buckets.agents.len() + buckets.conversations.len() > MAX_BUCKETS {
            prune(&mut buckets, now, &limits);
        }
        buckets.agents.insert(
            agent.to_owned(),
            Bucket {
                tokens: agent_bucket.tokens - 1.0,
                ..agent_bucket
            },
        );
        buckets.conversations.insert(
            conversation_key,
            Bucket {
                tokens: conversation_bucket.tokens - 1.0,
                ..conversation_bucket
            },
        );
        Ok(())
    }

    /// Takes one token from the shared `hello` bucket.
    pub fn check_hello(&self) -> Result<(), u64> {
        let now = Instant::now();
        let limits = self.limits;
        let mut buckets = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
        let bucket = buckets
            .hello
            .unwrap_or(Bucket {
                tokens: limits.hello_burst,
                updated: now,
            })
            .refilled(now, limits.hello_burst, limits.hello_refill_per_second);
        if bucket.tokens < 1.0 {
            buckets.hello = Some(bucket);
            return Err(bucket.wait_seconds(limits.hello_refill_per_second));
        }
        buckets.hello = Some(Bucket {
            tokens: bucket.tokens - 1.0,
            ..bucket
        });
        Ok(())
    }
}

/// Drops buckets that have refilled completely (they behave like new ones);
/// if that is not enough, the least recently used half goes too, so the
/// maps stay bounded (a dropped bucket starts full again).
fn prune(buckets: &mut Buckets, now: Instant, limits: &Limits) {
    drop_full_buckets(buckets, now, limits);
    if buckets.agents.len() + buckets.conversations.len() > MAX_BUCKETS {
        drop_oldest_half(&mut buckets.agents);
        drop_oldest_half(&mut buckets.conversations);
    }
}

fn drop_oldest_half<K: Eq + std::hash::Hash>(map: &mut HashMap<K, Bucket>) {
    let mut ages: Vec<Instant> = map.values().map(|bucket| bucket.updated).collect();
    ages.sort_unstable();
    if let Some(cutoff) = ages.get(ages.len() / 2).copied() {
        map.retain(|_, bucket| bucket.updated > cutoff);
    }
}

fn drop_full_buckets(buckets: &mut Buckets, now: Instant, limits: &Limits) {
    buckets.agents.retain(|_, bucket| {
        bucket
            .refilled(now, limits.agent_burst, limits.agent_refill_per_second)
            .tokens
            < limits.agent_burst
    });
    buckets.conversations.retain(|_, bucket| {
        bucket
            .refilled(
                now,
                limits.conversation_burst,
                limits.conversation_refill_per_second,
            )
            .tokens
            < limits.conversation_burst
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_conversation_burst_of_ten_then_retry_after() {
        let limiter = RateLimiter::new(Limits::default());
        for _ in 0..10 {
            assert_eq!(limiter.check_message("claude-code", "one"), Ok(()));
        }
        assert_eq!(limiter.check_message("claude-code", "one"), Err(1));
        // Another conversation of the same agent still has room.
        assert_eq!(limiter.check_message("claude-code", "two"), Ok(()));
    }

    #[test]
    fn an_agent_burst_of_thirty_across_conversations() {
        let limiter = RateLimiter::new(Limits::default());
        for index in 0..30 {
            assert_eq!(
                limiter.check_message("claude-code", &format!("c{}", index % 5)),
                Ok(())
            );
        }
        assert_eq!(limiter.check_message("claude-code", "fresh"), Err(1));
        assert_eq!(limiter.check_message("codex", "fresh"), Ok(()));
    }

    #[test]
    fn the_buckets_stay_bounded_even_when_none_has_refilled() {
        let limits = Limits {
            agent_refill_per_second: 0.0001,
            conversation_refill_per_second: 0.0001,
            ..Limits::default()
        };
        let limiter = RateLimiter::new(limits);
        for index in 0..(3 * MAX_BUCKETS) {
            let _ = limiter.check_message(&format!("agent-{index}"), "one");
        }
        let buckets = limiter
            .buckets
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        assert!(buckets.agents.len() + buckets.conversations.len() <= MAX_BUCKETS + 2);
    }

    #[test]
    fn a_refused_message_takes_no_token() {
        let limits = Limits {
            agent_burst: 2.0,
            conversation_burst: 1.0,
            agent_refill_per_second: 0.001,
            conversation_refill_per_second: 0.001,
            ..Limits::default()
        };
        let limiter = RateLimiter::new(limits);
        assert_eq!(limiter.check_message("a", "one"), Ok(()));
        assert!(limiter.check_message("a", "one").is_err());
        // The refused call left the agent bucket with its second token.
        assert_eq!(limiter.check_message("a", "two"), Ok(()));
    }
}
