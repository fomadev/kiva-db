// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Authorization for hierarchy metadata and managed data routing.

use crate::auth::{Priv, Role, User};
use crate::catalog::{Catalog, CollectionId, LegacyCollection, ProjectId, WorkspaceId};
use crate::engine::{BatchOp, Engine};
use crate::error::{KivaError, Result};
use crate::value::Value;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

const POLICY_KEY: &str = "catalog:hierarchy-grants";
pub(crate) type CachedPolicy = Option<(Option<u64>, Arc<Policy>)>;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "lowercase")]
pub(crate) enum Scope {
    Instance,
    Workspace(WorkspaceId),
    Project(ProjectId),
    Collection(CollectionId),
}

impl Scope {
    fn parent(self, catalog: &Catalog) -> Option<Self> {
        match self {
            Self::Instance => None,
            Self::Workspace(id) => catalog.workspace_name(id).map(|_| Self::Instance),
            Self::Project(id) => catalog.parent_workspace(id).map(Self::Workspace),
            Self::Collection(id) => catalog.parent_project(id).map(Self::Project),
        }
    }

    fn exists(self, catalog: &Catalog) -> bool {
        self == Self::Instance || self.parent(catalog).is_some()
    }

    fn includes(self, mut target: Self, catalog: &Catalog) -> bool {
        if !target.exists(catalog) {
            return false;
        }
        loop {
            if self == target {
                return true;
            }
            let Some(parent) = target.parent(catalog) else { return false };
            target = parent;
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Permission {
    Read,
    Write,
    Manage,
}

impl Permission {
    pub(crate) fn parse(name: &str) -> Result<Self> {
        match name {
            "read" => Ok(Self::Read),
            "write" => Ok(Self::Write),
            "manage" => Ok(Self::Manage),
            _ => Err(KivaError::Invalid("expected read, write or manage".into())),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
struct Grant {
    account: String,
    subject: String,
    scope: Scope,
    permission: Permission,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Policy {
    version: u32,
    grants: Vec<Grant>,
    #[serde(skip)]
    by_account: BTreeMap<String, BTreeMap<String, Arc<GrantSet>>>,
}

type GrantSet = BTreeSet<(Scope, Permission)>;

pub(crate) fn valid_subject(subject: &str) -> bool {
    subject.len() == 32 && subject.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

impl Policy {
    pub(crate) fn load(system: &Engine) -> Result<Self> {
        let Some((value, _)) = system.get(POLICY_KEY)? else {
            return Ok(Self { version: 1, grants: Vec::new(), by_account: BTreeMap::new() });
        };
        let Value::Doc(document) = value else {
            return Err(KivaError::Corrupted("hierarchy grants are not a document".into()));
        };
        let mut policy: Self = serde_json::from_value(document)
            .map_err(|error| KivaError::Corrupted(format!("invalid hierarchy grants: {error}")))?;
        if policy.version != 1 {
            return Err(KivaError::Format(format!("unsupported hierarchy grants version {}", policy.version)));
        }
        let mut unique = BTreeSet::new();
        if !policy.grants.iter().all(|grant| {
            crate::auth::valid_username(&grant.account) && valid_subject(&grant.subject) && unique.insert(grant)
        }) {
            return Err(KivaError::Corrupted("invalid or duplicate hierarchy grant".into()));
        }
        let mut by_account = BTreeMap::<String, BTreeMap<String, GrantSet>>::new();
        for grant in &policy.grants {
            by_account
                .entry(grant.account.clone())
                .or_default()
                .entry(grant.subject.clone())
                .or_default()
                .insert((grant.scope, grant.permission));
        }
        policy.by_account = by_account
            .into_iter()
            .map(|(name, subjects)| {
                let subjects = subjects.into_iter().map(|(subject, grants)| (subject, Arc::new(grants))).collect();
                (name, subjects)
            })
            .collect();
        Ok(policy)
    }

    pub(crate) fn load_cached(inst: &crate::instance::Instance) -> Result<std::sync::Arc<Self>> {
        let version = inst.system.meta(POLICY_KEY).map(|entry| entry.version);
        let mut cache =
            inst.managed_policy.lock().map_err(|_| KivaError::Other("managed policy lock poisoned".into()))?;
        if let Some((stored, policy)) = &*cache {
            if *stored == version {
                return Ok(policy.clone());
            }
        }
        let policy = std::sync::Arc::new(Self::load(&inst.system)?);
        *cache = Some((version, policy.clone()));
        Ok(policy)
    }

    /// The caller holds the instance's managed catalog lock.
    pub(crate) fn edit(system: &Engine, user: &User, scope: Scope, permission: Permission, add: bool) -> Result<()> {
        let mut policy = Self::load(system)?;
        let grant = Grant { account: user.name.clone(), subject: user.hierarchy_subject.clone(), scope, permission };
        policy.grants.retain(|entry| entry != &grant);
        if add {
            policy.grants.push(grant);
        }
        crate::crash_point!("hierarchy_grants.before_commit");
        system.apply_batch(
            vec![BatchOp::Put {
                key: POLICY_KEY.into(),
                value: Value::Doc(serde_json::to_value(policy)?),
                expires_at: 0,
                msgpack: false,
            }],
            None,
        )?;
        system.sync()?;
        crate::crash_point!("hierarchy_grants.after_commit");
        Ok(())
    }

    pub(crate) fn for_user(&self, user: &User) -> Access {
        Access {
            role: user.role,
            grants: self.by_account.get(&user.name).and_then(|subjects| subjects.get(&user.hierarchy_subject)).cloned(),
        }
    }
}

pub(crate) struct Access {
    role: Role,
    grants: Option<Arc<GrantSet>>,
}

impl Access {
    pub(crate) fn legacy(user: &User, aliases: &[LegacyCollection]) -> Self {
        let grants = aliases
            .iter()
            .filter_map(|alias| {
                let permission = match user.access(&alias.name)? {
                    Priv::Read => Permission::Read,
                    Priv::Write => Permission::Write,
                };
                Some((Scope::Collection(alias.id), permission))
            })
            .collect();
        Self { role: user.role, grants: Some(Arc::new(grants)) }
    }

    pub(crate) fn administrator() -> Self {
        Self { role: Role::Admin, grants: None }
    }

    pub(crate) fn role(&self) -> Role {
        self.role
    }

    pub(crate) fn allows(&self, catalog: &Catalog, target: Scope, permission: Permission) -> bool {
        if !target.exists(catalog) {
            return false;
        }
        if self.role == Role::Admin {
            return true;
        }
        if self.role == Role::Viewer && permission != Permission::Read {
            return false;
        }
        let Some(grants) = &self.grants else { return false };
        let mut scope = target;
        loop {
            if grants.contains(&(scope, permission))
                || (permission == Permission::Read && grants.contains(&(scope, Permission::Write)))
            {
                return true;
            }
            let Some(parent) = scope.parent(catalog) else { return false };
            scope = parent;
        }
    }

    pub(crate) fn visible(&self, catalog: &Catalog, target: Scope) -> bool {
        if !target.exists(catalog) {
            return false;
        }
        self.role == Role::Admin
            || self.grants.iter().flat_map(|grants| grants.iter()).any(|&(scope, permission)| {
                (self.role != Role::Viewer || permission != Permission::Manage)
                    && (scope.includes(target, catalog) || target.includes(scope, catalog))
            })
    }
}
