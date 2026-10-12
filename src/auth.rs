// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! User accounts. They are stored in KivaDB's own engine (the hidden system
//! database), one object per user under the key `user:<name>`. Passwords are
//! never stored: only a random salt and a PBKDF2-HMAC-SHA256 hash.

use crate::engine::Engine;
use crate::error::{KivaError, Result};
use crate::util::{hex_decode, hex_encode, now_secs, random_bytes};
use crate::value::Value;
use serde_json::{json, Value as Json};
use sha2::Sha256;
use std::collections::BTreeMap;

const ITERATIONS: u32 = 60_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Admin,
    Editor,
    Viewer,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Editor => "editor",
            Role::Viewer => "viewer",
        }
    }

    pub fn from_name(s: &str) -> Option<Role> {
        Some(match s.to_ascii_lowercase().as_str() {
            "admin" => Role::Admin,
            "editor" => Role::Editor,
            "viewer" => Role::Viewer,
            _ => return None,
        })
    }
}

/// Access level on a workspace.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Priv {
    Read,
    Write,
}

impl Priv {
    pub fn name(self) -> &'static str {
        match self {
            Priv::Read => "read",
            Priv::Write => "write",
        }
    }
}

#[derive(Clone, Debug)]
pub struct User {
    pub name: String,
    pub role: Role,
    /// Workspace name (or `*` for all) -> access level.
    pub grants: BTreeMap<String, Priv>,
    pub created: i64,
    salt: Vec<u8>,
    hash: Vec<u8>,
    iterations: u32,
}

fn hash_password(password: &str, salt: &[u8], iterations: u32) -> Vec<u8> {
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, iterations, &mut out);
    out.to_vec()
}

fn user_key(name: &str) -> String {
    format!("user:{name}")
}

pub fn valid_username(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

impl User {
    pub fn new(name: &str, password: &str, role: Role) -> User {
        let salt = random_bytes(16);
        let hash = hash_password(password, &salt, ITERATIONS);
        User {
            name: name.to_string(),
            role,
            grants: BTreeMap::new(),
            created: now_secs(),
            salt,
            hash,
            iterations: ITERATIONS,
        }
    }

    pub fn verify(&self, password: &str) -> bool {
        let h = hash_password(password, &self.salt, self.iterations);
        // Constant-time comparison.
        h.len() == self.hash.len() && h.iter().zip(&self.hash).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
    }

    pub fn has_empty_password(&self) -> bool {
        self.verify("")
    }

    pub fn set_password(&mut self, password: &str) {
        self.salt = random_bytes(16);
        self.iterations = ITERATIONS;
        self.hash = hash_password(password, &self.salt, ITERATIONS);
    }

    /// The user's personal workspace.
    pub fn home_workspace(&self) -> String {
        if self.role == Role::Admin {
            "default".to_string()
        } else {
            format!("u_{}", self.name)
        }
    }

    /// Access level this user has on `workspace`, if any.
    pub fn access(&self, workspace: &str) -> Option<Priv> {
        if self.role == Role::Admin || workspace == self.home_workspace() {
            return Some(Priv::Write);
        }
        let specific = self.grants.get(workspace).copied();
        let global = self.grants.get("*").copied();
        specific.max(global)
    }

    fn to_json(&self) -> Json {
        let grants: serde_json::Map<String, Json> =
            self.grants.iter().map(|(k, v)| (k.clone(), Json::String(v.name().into()))).collect();
        json!({
            "salt": hex_encode(&self.salt),
            "hash": hex_encode(&self.hash),
            "iterations": self.iterations,
            "role": self.role.name(),
            "grants": grants,
            "created": self.created,
        })
    }

    fn from_json(name: &str, j: &Json) -> Option<User> {
        let mut grants = BTreeMap::new();
        if let Some(m) = j["grants"].as_object() {
            for (k, v) in m {
                let p = match v.as_str() {
                    Some("write") => Priv::Write,
                    Some("read") => Priv::Read,
                    _ => continue,
                };
                grants.insert(k.clone(), p);
            }
        }
        Some(User {
            name: name.to_string(),
            role: Role::from_name(j["role"].as_str()?)?,
            grants,
            created: j["created"].as_i64().unwrap_or(0),
            salt: hex_decode(j["salt"].as_str()?)?,
            hash: hex_decode(j["hash"].as_str()?)?,
            iterations: j["iterations"].as_u64()? as u32,
        })
    }
}

pub fn get_user(sys: &Engine, name: &str) -> Result<Option<User>> {
    match sys.get(&user_key(name))? {
        Some((Value::Doc(j), _)) => Ok(User::from_json(name, &j)),
        _ => Ok(None),
    }
}

pub fn save_user(sys: &Engine, user: &User) -> Result<()> {
    sys.put(&user_key(&user.name), &Value::Doc(user.to_json()), 0, false)?;
    Ok(())
}

pub fn create_user(sys: &Engine, name: &str, password: &str, role: Role) -> Result<User> {
    if !valid_username(name) {
        return Err(KivaError::Invalid("user names may only contain letters, digits, '_', '-' and '.'".into()));
    }
    if get_user(sys, name)?.is_some() {
        return Err(KivaError::Exists(format!("user '{name}' already exists")));
    }
    let user = User::new(name, password, role);
    save_user(sys, &user)?;
    Ok(user)
}

pub fn delete_user(sys: &Engine, name: &str) -> Result<bool> {
    sys.delete(&user_key(name))
}

pub fn list_users(sys: &Engine) -> Result<Vec<User>> {
    let mut out = Vec::new();
    for (key, _) in sys.keys_matching("user:*", 0) {
        if let Some(u) = get_user(sys, &key["user:".len()..])? {
            out.push(u);
        }
    }
    Ok(out)
}

/// Creates the built-in `root` administrator (empty password) on first run.
pub fn ensure_root(sys: &Engine) -> Result<()> {
    if get_user(sys, "root")?.is_none() {
        save_user(sys, &User::new("root", "", Role::Admin))?;
    }
    Ok(())
}
