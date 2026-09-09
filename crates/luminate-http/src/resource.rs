// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Bounded resource-limit configuration and token buckets.

use std::collections::HashMap;
use std::hash::Hash;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sha2::{Digest as _, Sha256};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

pub(crate) const DEFAULT_MAXIMUM_HEADER_BYTES: usize = 32 * 1024;
pub(crate) const DEFAULT_MAXIMUM_HEADERS: usize = 64;
pub(crate) const DEFAULT_REQUEST_HEADER_TIMEOUT_SECONDS: u64 = 5;
pub(crate) const DEFAULT_REQUEST_TIMEOUT_SECONDS: u64 = 30;
pub(crate) const DEFAULT_KEEP_ALIVE_IDLE_SECONDS: u64 = 30;
pub(crate) const DEFAULT_TLS_HANDSHAKE_TIMEOUT_SECONDS: u64 = 10;
pub(crate) const DEFAULT_PUBLIC_REQUESTS_PER_MINUTE: u32 = 30;
pub(crate) const DEFAULT_PUBLIC_REQUEST_BURST: u32 = 10;
pub(crate) const DEFAULT_PRE_AUTH_REQUESTS_PER_MINUTE: u32 = 120;
pub(crate) const DEFAULT_PRE_AUTH_REQUEST_BURST: u32 = 40;
pub(crate) const DEFAULT_FAILED_AUTH_PER_MINUTE: u32 = 20;
pub(crate) const DEFAULT_FAILED_AUTH_BURST: u32 = 5;
pub(crate) const DEFAULT_FAILED_CREDENTIALS_PER_MINUTE: u32 = 10;
pub(crate) const DEFAULT_FAILED_CREDENTIAL_BURST: u32 = 3;
pub(crate) const DEFAULT_WEBSOCKET_UPGRADES_PER_MINUTE: u32 = 30;
pub(crate) const DEFAULT_WEBSOCKET_UPGRADE_BURST: u32 = 10;
pub(crate) const DEFAULT_MAXIMUM_WEBSOCKET_SESSIONS: usize = 64;
pub(crate) const DEFAULT_MAXIMUM_WEBSOCKET_MESSAGE_BYTES: usize = 1024 * 1024;
pub(crate) const DEFAULT_MAXIMUM_WEBSOCKET_QUEUE: usize = 16;
pub(crate) const DEFAULT_WEBSOCKET_START_TIMEOUT_SECONDS: u64 = 5;
pub(crate) const DEFAULT_WEBSOCKET_IDLE_SECONDS: u64 = 60;
pub(crate) const DEFAULT_RATE_LIMIT_ENTRIES: usize = 4096;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Rate {
    pub(crate) per_minute: u32,
    pub(crate) burst: u32,
}

impl Rate {
    fn capacity(self) -> u64 {
        u64::from(self.per_minute.saturating_add(self.burst)).saturating_mul(60_000)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct HttpLimits {
    pub(crate) maximum_header_bytes: usize,
    pub(crate) maximum_headers: usize,
    pub(crate) request_header_timeout: Duration,
    pub(crate) request_timeout: Duration,
    pub(crate) keep_alive_idle: Duration,
}

pub(crate) struct ResourceLimits {
    pub(crate) http: HttpLimits,
    pub(crate) maximum_websocket_message_bytes: usize,
    pub(crate) maximum_websocket_queue: usize,
    pub(crate) websocket_start_timeout: Duration,
    pub(crate) websocket_idle: Duration,
    pub(crate) pre_auth: RateStore<IpAddr>,
    pub(crate) public: RateStore<IpAddr>,
    pub(crate) failed_source: RateStore<IpAddr>,
    pub(crate) failed_credential: RateStore<[u8; 32]>,
    pub(crate) authenticated: RateStore<AuthenticatedRateKey>,
    pub(crate) websocket_upgrade: RateStore<IpAddr>,
    websocket_sessions: Arc<Semaphore>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum AuthenticatedRateKey {
    Direct {
        peer: IpAddr,
        credential_id: String,
    },
    Delegated {
        peer: IpAddr,
        frontend_credential_id: String,
        authority: String,
        subject: String,
    },
}

impl ResourceLimits {
    #[allow(
        clippy::too_many_arguments,
        reason = "all resource dimensions are assembled once from validated CLI configuration"
    )]
    pub(crate) fn new(
        http: HttpLimits,
        pre_auth: Rate,
        public: Rate,
        failed_source: Rate,
        failed_credential: Rate,
        authenticated: Rate,
        websocket_upgrade: Rate,
        maximum_entries: usize,
        maximum_websocket_sessions: usize,
        maximum_websocket_message_bytes: usize,
        maximum_websocket_queue: usize,
        websocket_start_timeout: Duration,
        websocket_idle: Duration,
    ) -> Self {
        Self {
            http,
            maximum_websocket_message_bytes,
            maximum_websocket_queue,
            websocket_start_timeout,
            websocket_idle,
            pre_auth: RateStore::new(pre_auth, maximum_entries),
            public: RateStore::new(public, maximum_entries),
            failed_source: RateStore::new(failed_source, maximum_entries),
            failed_credential: RateStore::new(failed_credential, maximum_entries),
            authenticated: RateStore::new(authenticated, maximum_entries),
            websocket_upgrade: RateStore::new(websocket_upgrade, maximum_entries),
            websocket_sessions: Arc::new(Semaphore::new(maximum_websocket_sessions)),
        }
    }

    pub(crate) fn websocket_session(&self) -> Option<OwnedSemaphorePermit> {
        Arc::clone(&self.websocket_sessions)
            .try_acquire_owned()
            .ok()
    }
}

#[derive(Clone, Copy, Debug)]
struct Window {
    updated: Instant,
    token_millis: u64,
    sequence: u64,
}

impl Window {
    fn refill(&mut self, rate: Rate, now: Instant) {
        let elapsed =
            u64::try_from(now.duration_since(self.updated).as_millis()).unwrap_or(u64::MAX);
        self.token_millis = self
            .token_millis
            .saturating_add(elapsed.saturating_mul(u64::from(rate.per_minute)))
            .min(rate.capacity());
        self.updated = now;
    }
}

pub(crate) struct RateStore<K> {
    rate: Rate,
    maximum_entries: usize,
    inner: Mutex<Store<K>>,
}

struct Store<K> {
    entries: HashMap<K, Window>,
    next_sequence: u64,
}

impl<K> RateStore<K>
where
    K: Clone + Eq + Hash,
{
    fn new(rate: Rate, maximum_entries: usize) -> Self {
        Self {
            rate,
            maximum_entries,
            inner: Mutex::new(Store {
                entries: HashMap::new(),
                next_sequence: 0,
            }),
        }
    }

    pub(crate) async fn allow(&self, key: K) -> bool {
        self.allow_at(key, Instant::now()).await
    }

    pub(crate) async fn available(&self, key: K) -> bool {
        self.available_at(key, Instant::now()).await
    }

    async fn available_at(&self, key: K, now: Instant) -> bool {
        let mut store = self.inner.lock().await;
        store.entries.get_mut(&key).is_none_or(|window| {
            window.refill(self.rate, now);
            window.token_millis >= 60_000
        })
    }

    async fn allow_at(&self, key: K, now: Instant) -> bool {
        let mut store = self.inner.lock().await;
        store
            .entries
            .retain(|_, window| now.duration_since(window.updated) < Duration::from_secs(60));
        if !store.entries.contains_key(&key)
            && store.entries.len() >= self.maximum_entries
            && let Some(oldest) = store
                .entries
                .iter()
                .min_by_key(|(_, window)| window.sequence)
                .map(|(key, _)| key.clone())
        {
            store.entries.remove(&oldest);
        }
        let sequence = store.next_sequence;
        store.next_sequence = store.next_sequence.wrapping_add(1);
        let window = store.entries.entry(key).or_insert(Window {
            updated: now,
            token_millis: self.rate.capacity(),
            sequence,
        });
        window.refill(self.rate, now);
        window.sequence = sequence;
        if window.token_millis < 60_000 {
            return false;
        }
        window.token_millis -= 60_000;
        true
    }
}

pub(crate) fn credential_digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[cfg(test)]
mod tests {
    use super::{Rate, RateStore};
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn availability_refills_without_consuming_tokens() {
        let store = RateStore::new(
            Rate {
                per_minute: 2,
                burst: 1,
            },
            2,
        );
        let now = Instant::now();
        assert!(store.available_at("a", now).await);
        for _ in 0..3 {
            assert!(store.allow_at("a", now).await);
        }
        assert!(!store.allow_at("a", now).await);
        assert!(!store.available_at("a", now).await);

        let halfway = now + Duration::from_secs(15);
        assert!(!store.available_at("a", halfway).await);
        assert!(!store.allow_at("a", halfway).await);

        let refilled = now + Duration::from_secs(30);
        assert!(store.available_at("a", refilled).await);
        assert!(store.available_at("a", refilled).await);
        assert!(store.allow_at("a", refilled).await);
        assert!(!store.allow_at("a", refilled).await);
    }

    #[tokio::test]
    async fn refill_is_capped_at_capacity() {
        let store = RateStore::new(
            Rate {
                per_minute: 2,
                burst: 1,
            },
            2,
        );
        let now = Instant::now();
        assert!(store.allow_at("a", now).await);
        let later = now + Duration::from_secs(59);
        assert!(store.available_at("a", later).await);
        for _ in 0..3 {
            assert!(store.allow_at("a", later).await);
        }
        assert!(!store.allow_at("a", later).await);
    }

    #[tokio::test]
    async fn zero_rate_does_not_refill_a_consumed_burst() {
        let store = RateStore::new(
            Rate {
                per_minute: 0,
                burst: 1,
            },
            2,
        );
        let now = Instant::now();
        assert!(store.allow_at("a", now).await);
        let later = now + Duration::from_secs(59);
        assert!(!store.available_at("a", later).await);
        assert!(!store.allow_at("a", later).await);
    }

    #[tokio::test]
    async fn bounded_store_evicts_the_oldest_entry_deterministically() {
        let store = RateStore::new(
            Rate {
                per_minute: 1,
                burst: 0,
            },
            2,
        );
        let now = Instant::now();
        assert!(store.allow_at("a", now).await);
        assert!(store.allow_at("b", now).await);
        assert!(store.allow_at("c", now).await);
        assert!(store.allow_at("a", now).await);
        assert!(!store.allow_at("c", now + Duration::from_millis(1)).await);
    }
}
