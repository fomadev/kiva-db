// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! User accounts. They are stored in KivaDB's own engine (the hidden system
//! database), one object per user under the key `user:<name>`. Passwords are
//! never stored: only a random salt and a PBKDF2-HMAC-SHA256 hash.
//!
//! An account reaches nothing by itself: an administrator gives it access to
//! workspaces with grants. A grant names a scope: one workspace (`analytics`,
//! `travel.places`), every workspace of a project (`travel.*`), the workspaces of
//! that name in every project (`*.places`), or everything (`*`).

use crate::engine::Engine;
use crate::error::{KivaError, Result};
use crate::util::{hex_decode, hex_encode, now_secs, random_bytes};
use crate::value::Value;
use serde_json::{json, Value as Json};
use sha2::Sha256;
use std::collections::BTreeMap;

const ITERATIONS: u32 = 60_000;

/// Version of the account model stored with each user. Accounts saved
/// before 2.0 (no `model`) had a personal workspace `u_<name>` reached
/// without a grant; [`migrate_personal_workspaces`] turns it into one.
const MODEL: u64 = 2;

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
    /// Scope (`*`, `travel.*`, `travel.places`...) -> access level.
    pub grants: BTreeMap<String, Priv>,
    /// The workspace a session enters at login, when the user can reach it.
    pub default_workspace: Option<String>,
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
            default_workspace: None,
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

    /// Access level this user has on `workspace`, if any: the highest of
    /// the grants whose scope covers it.
    pub fn access(&self, workspace: &str) -> Option<Priv> {
        if self.role == Role::Admin {
            return Some(Priv::Write);
        }
        self.grants.iter().filter(|(scope, _)| scope_covers(scope, workspace)).map(|(_, p)| *p).max()
    }

    /// Whether the user may run the maintenance of `workspace` and create
    /// workspaces next to it: an administrator, or a user who can write in
    /// every workspace of its project (`travel.*`, or `*`).
    pub fn manages(&self, workspace: &str) -> bool {
        self.role == Role::Admin || project_of(workspace).is_some_and(|p| self.manages_project(p))
    }

    /// Whether the user can write in every workspace of `project`.
    pub fn manages_project(&self, project: &str) -> bool {
        match self.role {
            Role::Admin => true,
            Role::Viewer => false,
            Role::Editor => {
                [format!("{project}.*"), "*".to_string()].iter().any(|s| self.grants.get(s) == Some(&Priv::Write))
            }
        }
    }

    /// The workspace a login enters, among the `existing` ones: the default
    /// workspace when the user can reach it, `default` for an administrator,
    /// otherwise the only workspace the user can reach, if there is just one.
    pub fn arrival(&self, existing: &[String]) -> Option<String> {
        if let Some(w) = &self.default_workspace {
            if existing.contains(w) && self.access(w).is_some() {
                return Some(w.clone());
            }
        }
        if self.role == Role::Admin {
            return Some("default".to_string());
        }
        let mut reachable = existing.iter().filter(|w| self.access(w).is_some());
        match (reachable.next(), reachable.next()) {
            (Some(only), None) => Some(only.clone()),
            _ => None,
        }
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
            "default_workspace": self.default_workspace,
            "created": self.created,
            "model": MODEL,
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
            default_workspace: j["default_workspace"].as_str().map(str::to_string),
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

/// Name of the built-in administrator account.
pub const DEFAULT_ADMIN: &str = "kiva";

/// Creates the built-in `kiva` administrator (empty password) on first run.
/// Homes created when that account was still called `root` are migrated:
/// the account is renamed and keeps its password.
pub fn ensure_admin(sys: &Engine) -> Result<()> {
    if get_user(sys, DEFAULT_ADMIN)?.is_some() {
        return Ok(());
    }
    match get_user(sys, "root")? {
        Some(mut legacy) => {
            legacy.name = DEFAULT_ADMIN.to_string();
            legacy.role = Role::Admin;
            save_user(sys, &legacy)?;
            delete_user(sys, "root")?;
        }
        None => save_user(sys, &User::new(DEFAULT_ADMIN, "", Role::Admin))?,
    }
    Ok(())
}

/// Accounts created before 2.0 reached their personal workspace `u_<name>`
/// without any grant. Each one keeps it: a write grant on it, and it becomes
/// the account's default workspace, so applications connected as that user
/// land where their data is. Runs at startup; an account is migrated once.
pub fn migrate_personal_workspaces(sys: &Engine, exists: impl Fn(&str) -> bool) -> Result<usize> {
    let mut migrated = 0;
    for (key, _) in sys.keys_matching("user:*", 0) {
        let Some((Value::Doc(j), _)) = sys.get(&key)? else { continue };
        if j.get("model").is_some() {
            continue;
        }
        let Some(mut user) = User::from_json(&key["user:".len()..], &j) else { continue };
        let personal = format!("u_{}", user.name);
        if user.role != Role::Admin && exists(&personal) {
            user.grants.entry(personal.clone()).or_insert(Priv::Write);
            user.default_workspace.get_or_insert(personal);
            migrated += 1;
        }
        save_user(sys, &user)?;
    }
    Ok(migrated)
}

/// A project or workspace name, or one of the two parts of `travel.places`.
pub fn valid_name_part(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
}

/// `analytics` or `travel.places`: a workspace, alone or in a project.
pub fn valid_workspace_name(name: &str) -> bool {
    match name.split_once('.') {
        Some((project, rest)) => valid_name_part(project) && valid_name_part(rest),
        None => valid_name_part(name),
    }
}

/// The project of a workspace: `travel` for `travel.places`, none for `analytics`.
pub fn project_of(workspace: &str) -> Option<&str> {
    workspace.split_once('.').map(|(p, _)| p)
}

/// Checks a grant scope and writes it the one way it is stored: `*` (also
/// written `*.*` or `all`), `<project>.*`, `*.<workspace>`,
/// `<project>.<workspace>` or `<workspace>`.
pub fn normalize_scope(scope: &str) -> std::result::Result<String, String> {
    let s = scope.trim();
    if s == "*" || s == "*.*" || s.eq_ignore_ascii_case("all") {
        return Ok("*".into());
    }
    let ok = match s.split_once('.') {
        Some((p, w)) => (p == "*" || valid_name_part(p)) && (w == "*" || valid_name_part(w)),
        None => valid_name_part(s),
    };
    if ok {
        Ok(s.to_string())
    } else {
        Err(format!(
            "'{scope}' is not a scope: write a workspace (places, travel.places), the workspaces of a project \
             (travel.*), a workspace of every project (*.places) or everything (*.*)"
        ))
    }
}

/// Whether a grant on `scope` covers `workspace`.
pub fn scope_covers(scope: &str, workspace: &str) -> bool {
    if scope == "*" {
        return true;
    }
    match (scope.split_once('.'), workspace.split_once('.')) {
        (Some((sp, sw)), Some((p, w))) => (sp == "*" || sp == p) && (sw == "*" || sw == w),
        (None, None) => scope == workspace,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_cover_projects_and_workspaces() {
        for (scope, ws, covered) in [
            ("*", "default", true),
            ("*", "travel.places", true),
            ("travel.*", "travel.places", true),
            ("travel.*", "school.places", false),
            ("travel.*", "places", false),
            ("*.places", "school.places", true),
            ("*.places", "school.cache", false),
            ("travel.places", "travel.places", true),
            ("places", "travel.places", false),
            ("places", "places", true),
        ] {
            assert_eq!(scope_covers(scope, ws), covered, "{scope} on {ws}");
        }
        assert_eq!(normalize_scope("*.*").unwrap(), "*");
        assert_eq!(normalize_scope("ALL").unwrap(), "*");
        assert_eq!(normalize_scope("travel.*").unwrap(), "travel.*");
        assert!(normalize_scope("a.b.c").is_err() && normalize_scope("ba ak").is_err());
        assert!(
            valid_workspace_name("travel.places") && !valid_workspace_name("travel.") && !valid_workspace_name("a.b.c")
        );
    }

    #[test]
    fn a_login_enters_the_default_or_the_only_workspace() {
        let all: Vec<String> = ["default", "travel.places", "travel.cache", "school.places"].map(String::from).to_vec();
        let mut u = User::new("dev", "pw", Role::Editor);
        assert_eq!(u.arrival(&all), None);
        u.grants.insert("school.places".into(), Priv::Read);
        assert_eq!(u.arrival(&all).as_deref(), Some("school.places"));
        u.grants.insert("travel.*".into(), Priv::Write);
        assert_eq!(u.arrival(&all), None);
        u.default_workspace = Some("travel.cache".into());
        assert_eq!(u.arrival(&all).as_deref(), Some("travel.cache"));
        assert!(u.manages("travel.cache") && !u.manages("school.places") && !u.manages("default"));
        assert_eq!(u.access("travel.places"), Some(Priv::Write));
        assert_eq!(u.access("default"), None);
    }
}
