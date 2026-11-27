// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.

use crate::auth::User;
use crate::error::{KivaError, Result};
use crate::util::{fill_random, hex_encode, now_secs};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type Fingerprint = [u8; 32];

pub(crate) const INVALID: &str = "invalid HTTP session";

pub(crate) fn invalid() -> KivaError {
    KivaError::Denied(INVALID.into())
}

pub(crate) struct Lease {
    fingerprint: Fingerprint,
    pub(crate) account: String,
    subject: String,
    generation: Fingerprint,
    created_at: i64,
    expires_at: i64,
    deadline: Instant,
    revoked: AtomicBool,
}

impl Lease {
    pub(crate) fn matches(&self, user: &User) -> bool {
        self.active_at(Instant::now())
            && self.account == user.name
            && self.subject == user.hierarchy_subject
            && self.generation == user.credential_generation()
    }

    fn active_at(&self, now: Instant) -> bool {
        now < self.deadline && !self.revoked.load(Ordering::Acquire)
    }

    pub(crate) fn info(&self) -> Value {
        json!({"account": self.account, "created_at": self.created_at, "expires_at": self.expires_at})
    }
}

#[derive(Default)]
struct State {
    sessions: HashMap<Fingerprint, Arc<Lease>>,
    issued: u64,
    revoked: u64,
    expired: u64,
    refused: u64,
}

impl State {
    fn prune(&mut self, now: Instant) {
        let before = self.sessions.len();
        self.sessions.retain(|_, lease| lease.active_at(now));
        self.expired += (before - self.sessions.len()) as u64;
    }

    fn revoke(&mut self, lease: &Lease) {
        if let Some(active) = self.sessions.remove(&lease.fingerprint) {
            active.revoked.store(true, Ordering::Release);
            self.revoked += 1;
        }
    }
}

pub(crate) struct Registry {
    state: Mutex<State>,
    limit: usize,
    per_account: usize,
    ttl: Duration,
}

impl Registry {
    pub(crate) fn new(limit: usize, per_account: usize, ttl: Duration) -> Self {
        Self { state: Mutex::default(), limit, per_account, ttl }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, State>> {
        self.state.lock().map_err(|_| KivaError::Other("HTTP session registry poisoned".into()))
    }

    pub(crate) fn issue(&self, user: &User) -> Result<(String, Arc<Lease>)> {
        let mut random = [0u8; 32];
        fill_random(&mut random)?;
        let token = hex_encode(&random);
        let fingerprint = Sha256::digest(token.as_bytes()).into();
        let mut state = self.lock()?;
        let now = Instant::now();
        state.prune(now);
        let generation = user.credential_generation();
        let before = state.sessions.len();
        state.sessions.retain(|_, lease| {
            if lease.account != user.name || (lease.subject == user.hierarchy_subject && lease.generation == generation)
            {
                return true;
            }
            lease.revoked.store(true, Ordering::Release);
            false
        });
        state.revoked += (before - state.sessions.len()) as u64;
        let account_count = state.sessions.values().filter(|lease| lease.subject == user.hierarchy_subject).count();
        if state.sessions.len() >= self.limit || account_count >= self.per_account {
            state.refused += 1;
            return Err(KivaError::Other("ERR_SESSION_LIMIT: HTTP session capacity reached".into()));
        }
        if state.sessions.contains_key(&fingerprint) {
            return Err(KivaError::Other("HTTP session identifier collision".into()));
        }
        let created_at = now_secs();
        let lease = Arc::new(Lease {
            fingerprint,
            account: user.name.clone(),
            subject: user.hierarchy_subject.clone(),
            generation,
            created_at,
            expires_at: created_at + self.ttl.as_secs() as i64,
            deadline: now + self.ttl,
            revoked: AtomicBool::new(false),
        });
        state.sessions.insert(fingerprint, lease.clone());
        state.issued += 1;
        Ok((token, lease))
    }

    pub(crate) fn lookup(&self, token: &str) -> Result<Arc<Lease>> {
        if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            return Err(invalid());
        }
        let fingerprint: Fingerprint = Sha256::digest(token.as_bytes()).into();
        let mut state = self.lock()?;
        let lease = state.sessions.get(&fingerprint).cloned().ok_or_else(invalid)?;
        if !lease.active_at(Instant::now()) {
            state.sessions.remove(&fingerprint);
            state.expired += 1;
            return Err(invalid());
        }
        Ok(lease)
    }

    pub(crate) fn logout(&self, lease: &Lease) -> Result<()> {
        self.lock()?.revoke(lease);
        Ok(())
    }

    pub(crate) fn revoke_account(&self, name: &str) -> Result<usize> {
        let mut state = self.lock()?;
        state.prune(Instant::now());
        let before = state.sessions.len();
        state.sessions.retain(|_, lease| {
            if lease.account != name {
                return true;
            }
            lease.revoked.store(true, Ordering::Release);
            false
        });
        let revoked = before - state.sessions.len();
        state.revoked += revoked as u64;
        Ok(revoked)
    }

    pub(crate) fn metrics(&self) -> Result<Value> {
        let mut state = self.lock()?;
        state.prune(Instant::now());
        Ok(json!({
            "active": state.sessions.len(), "issued": state.issued, "revoked": state.revoked,
            "expired": state.expired, "refused": state.refused,
            "capacity": self.limit, "per_account": self.per_account, "ttl_seconds": self.ttl.as_secs()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expired_entries_are_reclaimed_on_issue_and_lookup() {
        let registry = Registry::new(1, 1, Duration::ZERO);
        let user = User::new("viewer", "example-password", crate::auth::Role::Viewer).unwrap();
        let (first, lease) = registry.issue(&user).unwrap();
        assert!(!lease.matches(&user));
        let (second, _) = registry.issue(&user).unwrap();
        assert_ne!(first, second);
        assert!(registry.lookup(&second).is_err());
        let metrics = registry.metrics().unwrap();
        assert_eq!(metrics["expired"], 2);
        assert_eq!(metrics["active"], 0);
    }
}
