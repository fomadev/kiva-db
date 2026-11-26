// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Administrative metadata preview. Collection identities are never passed to
//! the historical storage engine or its authorization and replication paths.

use std::sync::{Arc, RwLockReadGuard, RwLockWriteGuard};

use crate::auth::{self, Role, User};
use crate::catalog::{Catalog, CatalogStore, CollectionId, ProjectId, WorkspaceId};
use crate::error::{KivaError, Result};
use crate::exec::Out;
use crate::hierarchy_auth::{Access, Permission, Policy, Scope};
use crate::lexer::Token;
use crate::session::{Need, Session};

#[derive(Clone, Copy)]
pub(crate) struct Context {
    workspace: WorkspaceId,
    project: Option<ProjectId>,
    collection: Option<CollectionId>,
    managed: bool,
}

impl Context {
    fn workspace(workspace: WorkspaceId, managed: bool) -> Self {
        Self { workspace, project: None, collection: None, managed }
    }

    fn normalize(&mut self, catalog: &Catalog) {
        if catalog.workspace_name(self.workspace).is_none() {
            *self = Self::workspace(catalog.workspace("default").expect("validated default workspace"), self.managed);
        } else if self.project.is_some_and(|id| catalog.project_name(id).is_none()) {
            *self = Self::workspace(self.workspace, self.managed);
        } else if self.collection.is_some_and(|id| catalog.collection_name(id).is_none()) {
            self.collection = None;
        }
    }

    fn names(self, catalog: &Catalog) -> [&str; 3] {
        [
            catalog.workspace_name(self.workspace).unwrap_or("default"),
            self.project.and_then(|id| catalog.project_name(id)).unwrap_or(""),
            self.collection.and_then(|id| catalog.collection_name(id)).unwrap_or(""),
        ]
    }

    fn scope(self) -> Scope {
        self.collection
            .map(Scope::Collection)
            .or_else(|| self.project.map(Scope::Project))
            .unwrap_or(Scope::Workspace(self.workspace))
    }

    fn visible_names(self, catalog: &Catalog, access: &Access) -> [String; 3] {
        let scopes = [
            Some(Scope::Workspace(self.workspace)),
            self.project.map(Scope::Project),
            self.collection.map(Scope::Collection),
        ];
        let names = self.names(catalog);
        std::array::from_fn(|i| {
            if scopes[i].is_some_and(|scope| access.visible(catalog, scope)) {
                names[i].to_owned()
            } else {
                String::new()
            }
        })
    }

    fn path(self, catalog: &Catalog, access: &Access) -> String {
        self.visible_names(catalog, access).into_iter().filter(|name| !name.is_empty()).collect::<Vec<_>>().join(".")
    }
}

#[derive(Clone, Copy)]
enum Parent {
    Root,
    Workspace(WorkspaceId),
    Project(ProjectId),
}

#[derive(Clone, Copy)]
enum Target {
    Workspace(WorkspaceId),
    Project(ProjectId),
    Collection(CollectionId),
}

impl Parent {
    fn for_kind(kind: &str, context: Context) -> Result<Self> {
        match kind {
            "workspace" => Ok(Self::Root),
            "project" => Ok(Self::Workspace(context.workspace)),
            "collection" => context.project.map(Self::Project).ok_or_else(|| {
                KivaError::Invalid("ERR_NO_PROJECT_SELECTED: select a project first with use <project>".into())
            }),
            _ => Err(KivaError::Denied("only hierarchy metadata commands are available in preview".into())),
        }
    }

    fn scope(self) -> Scope {
        match self {
            Self::Root => Scope::Instance,
            Self::Workspace(id) => Scope::Workspace(id),
            Self::Project(id) => Scope::Project(id),
        }
    }

    fn target(self, catalog: &Catalog, name: &str) -> Option<Target> {
        match self {
            Self::Root => catalog.workspace(name).map(Target::Workspace),
            Self::Workspace(parent) => catalog.project(parent, name).map(Target::Project),
            Self::Project(parent) => catalog.collection(parent, name).map(Target::Collection),
        }
    }

    fn names(self, catalog: &Catalog) -> Vec<&str> {
        let mut names: Vec<_> = match self {
            Self::Root => catalog.workspace_names().collect(),
            Self::Workspace(parent) => catalog.project_names(parent).collect(),
            Self::Project(parent) => catalog.collection_names(parent).collect(),
        };
        names.sort_unstable();
        names
    }

    fn create(self, catalog: &mut Catalog, name: &str) -> Result<()> {
        crate::hierarchy_layout::segment(name)?;
        match self {
            Self::Root => catalog.create_workspace(name).map(|_| ()),
            Self::Workspace(parent) => catalog.create_project(parent, name).map(|_| ()),
            Self::Project(parent) => catalog.create_collection(parent, name).map(|_| ()),
        }
    }
}

impl Target {
    fn scope(self) -> Scope {
        match self {
            Self::Workspace(id) => Scope::Workspace(id),
            Self::Project(id) => Scope::Project(id),
            Self::Collection(id) => Scope::Collection(id),
        }
    }

    fn rename(self, catalog: &mut Catalog, name: &str) -> Result<()> {
        crate::hierarchy_layout::segment(name)?;
        match self {
            Self::Workspace(id) => catalog.rename_workspace(id, name),
            Self::Project(id) => catalog.rename_project(id, name),
            Self::Collection(id) => catalog.rename_collection(id, name),
        }
    }

    fn drop_from(self, catalog: &mut Catalog, cascade: bool) -> Result<()> {
        match self {
            Self::Workspace(id) => catalog.drop_workspace(id, cascade).map(|_| ()),
            Self::Project(id) => catalog.drop_project(id, cascade),
            Self::Collection(_) if cascade => Err(KivaError::Invalid("collections do not support cascade".into())),
            Self::Collection(id) => {
                catalog.drop_collection(id);
                Ok(())
            }
        }
    }

    fn info(self, catalog: &Catalog) -> String {
        match self {
            Self::Workspace(id) => format!(
                "Workspace {} ({} projects)",
                catalog.workspace_name(id).unwrap(),
                catalog.project_names(id).count()
            ),
            Self::Project(id) => format!(
                "Project {} ({} collections)",
                catalog.project_name(id).unwrap(),
                catalog.collection_names(id).count()
            ),
            Self::Collection(id) => format!("Collection {} (metadata only)", catalog.collection_name(id).unwrap()),
        }
    }
}

fn hidden() -> KivaError {
    KivaError::Denied("hierarchy resource or operation is unavailable".into())
}

fn grant_scope(catalog: &Catalog, kind: &Token, args: &[Token]) -> Result<Scope> {
    if kind.is_kw("instance") && args.is_empty() {
        return Ok(Scope::Instance);
    }
    if let [path] = args {
        let parts: Vec<_> = path.text.split('.').collect();
        match parts.as_slice() {
            [name] if kind.is_kw("workspace") => {
                return Ok(Scope::Workspace(lookup(catalog.workspace(name), "workspace", name)?))
            }
            [workspace, project] if kind.is_kw("project") => {
                let workspace = lookup(catalog.workspace(workspace), "workspace", workspace)?;
                return Ok(Scope::Project(lookup(catalog.project(workspace, project), "project", project)?));
            }
            [workspace, project, collection] if kind.is_kw("collection") => {
                return Ok(Scope::Collection(catalog.resolve_collection(workspace, project, collection)?));
            }
            _ => {}
        }
    }
    Err(KivaError::Invalid("expected instance, workspace <w>, project <w.p> or collection <w.p.c>".into()))
}

fn lookup<T>(found: Option<T>, kind: &str, name: &str) -> Result<T> {
    found.ok_or_else(|| KivaError::NotFound(format!("{kind} '{name}' does not exist")))
}

fn navigate(catalog: &Catalog, mut current: Context, command: &str, args: &[Token]) -> Result<Context> {
    match (command, args) {
        ("switch", [name]) => {
            return Ok(Context::workspace(
                lookup(catalog.workspace(&name.text), "workspace", &name.text)?,
                current.managed,
            ))
        }
        ("use", [name]) if !name.text.contains('.') => {
            current.project = Some(lookup(catalog.project(current.workspace, &name.text), "project", &name.text)?);
            current.collection = None;
            return Ok(current);
        }
        _ => {}
    }
    let (name, explicit) = match (command, args) {
        ("use", [name]) => (name.text.as_str(), false),
        ("use", [kind, name]) if kind.is_kw("collection") => (name.text.as_str(), true),
        _ => return Err(KivaError::Invalid("use switch <workspace>, use <project>, or use collection <path>".into())),
    };
    let parts: Vec<_> = name.split('.').collect();
    let (workspace, project, collection) = match parts.as_slice() {
        [collection] if explicit => (
            current.workspace,
            current.project.ok_or_else(|| {
                KivaError::Invalid("ERR_NO_PROJECT_SELECTED: select a project first with use <project>".into())
            })?,
            *collection,
        ),
        [project, collection] => {
            (current.workspace, lookup(catalog.project(current.workspace, project), "project", project)?, *collection)
        }
        [workspace, project, collection] if explicit => {
            let ws = lookup(catalog.workspace(workspace), "workspace", workspace)?;
            (ws, lookup(catalog.project(ws, project), "project", project)?, *collection)
        }
        _ => return Err(KivaError::Invalid(format!("invalid collection path '{name}'"))),
    };
    let id = lookup(catalog.collection(project, collection), "collection", collection)?;
    Ok(Context { workspace, project: Some(project), collection: Some(id), managed: current.managed })
}

enum LayoutGuard<'a> {
    Shared(#[allow(dead_code)] RwLockReadGuard<'a, ()>),
    Exclusive(#[allow(dead_code)] RwLockWriteGuard<'a, ()>),
}

impl Session {
    fn hierarchy_user(&self) -> Result<User> {
        let ctx =
            self.user.as_ref().ok_or_else(|| KivaError::Denied("hierarchy preview requires authentication".into()))?;
        if !self.local && !self.inst.auth_required() {
            return Err(KivaError::Denied("hierarchy preview requires TCP authentication".into()));
        }
        let user = auth::get_user(&self.inst.system, &ctx.name)?.ok_or_else(hidden)?;
        let valid = self.hierarchy_subject.as_ref().map_or_else(
            || ctx.role == Role::Admin && user.role == Role::Admin && !self.inst.auth_required(),
            |subject| subject == &user.hierarchy_subject,
        );
        if !valid {
            return Err(hidden());
        }
        Ok(user)
    }

    fn hierarchy_access(&self, user: &User, managed: bool) -> Result<Access> {
        if managed {
            return Ok(Policy::load_cached(&self.inst)?.for_user(user));
        }
        if let Some(aliases) = crate::migration::staged_aliases(&self.inst.home)? {
            return Ok(Access::legacy(user, &aliases));
        }
        if user.role == Role::Admin {
            return Ok(Access::administrator());
        }
        Err(KivaError::Denied("the metadata-only preview requires an administrator".into()))
    }

    pub(crate) fn hierarchy_location(&self) -> Result<Option<([String; 3], Role)>> {
        let Some(mut context) = self.hierarchy else { return Ok(None) };
        let _guard = lock_layout(&self.inst, context.managed, false)?;
        let user = self.hierarchy_user()?;
        let catalog = self.context_catalog(context.managed)?;
        let access = self.hierarchy_access(&user, context.managed)?;
        context.normalize(&catalog);
        Ok(Some((context.visible_names(&catalog, &access), access.role())))
    }
}

fn lock_layout(inst: &crate::instance::Instance, managed: bool, exclusive: bool) -> Result<Option<LayoutGuard<'_>>> {
    if !managed {
        return Ok(None);
    }
    let lock = &inst.managed_layout_lock;
    let guard = if exclusive {
        lock.write().map(LayoutGuard::Exclusive).map_err(|_| ())
    } else {
        lock.read().map(LayoutGuard::Shared).map_err(|_| ())
    };
    guard.map(Some).map_err(|_| KivaError::Other("managed layout lock poisoned".into()))
}

impl Session {
    pub(crate) fn execute_hierarchy(
        &mut self,
        command: &str,
        args: &[Token],
        out: &mut Out,
    ) -> std::result::Result<(), String> {
        self.hierarchy_command(command, args, out).map_err(|error| error.to_string())
    }

    fn hierarchy_command(&mut self, command: &str, args: &[Token], out: &mut Out) -> Result<()> {
        let managed = self.hierarchy.is_some_and(|context| context.managed)
            || command == "hierarchy" && args.first().is_some_and(|mode| mode.is_kw("storage") || mode.is_kw("data"));
        let native_account = self.native_hierarchy
            && matches!(command, "create" | "drop")
            && args.first().is_some_and(|token| token.is_kw("user"));
        let native_shutdown = self.native_hierarchy && command == "shutdown" && args.is_empty();
        let exclusive = if native_account || native_shutdown {
            true
        } else if command == "hierarchy" {
            args.first()
                .is_some_and(|token| ["storage", "data", "grant", "revoke"].iter().any(|word| token.is_kw(word)))
        } else {
            matches!(command, "workspace" | "project" | "collection" | "collections")
                && args.first().is_some_and(|token| {
                    ["create", "rename", "drop"].iter().any(|word| token.text.eq_ignore_ascii_case(word))
                })
        };
        let lock_session = self.inst.clone();
        let _guard = lock_layout(&lock_session, managed, exclusive)?;
        let user = self.hierarchy_user()?;
        if self.txn.is_some() {
            return Err(KivaError::Invalid("hierarchy preview is unavailable during a transaction".into()));
        }
        if native_shutdown {
            if user.role != Role::Admin {
                return Err(hidden());
            }
            self.inst.audit.log(self.user_name(), "shutdown", true);
            self.inst.stop_requested.store(true, std::sync::atomic::Ordering::SeqCst);
            out.say("KivaDB is shutting down.");
            out.action = crate::exec::Action::Exit;
            return Ok(());
        }
        if native_account {
            if user.role != Role::Admin {
                return Err(hidden());
            }
            let map_error = KivaError::Other;
            return match (command, args) {
                ("drop", [_, _]) => self.cmd_drop(args, out).map_err(map_error),
                ("create", [_, _, identified, by, password, rest @ ..])
                    if identified.is_kw("identified")
                        && by.is_kw("by")
                        && !password.text.is_empty()
                        && (rest.is_empty() || matches!(rest, [role, _] if role.is_kw("role"))) =>
                {
                    self.cmd_create(args, out).map_err(map_error)
                }
                _ => Err(KivaError::Invalid(
                    "use create user <name> identified by <password> [role <role>] or drop user <name>".into(),
                )),
            };
        }
        if command == "hierarchy" {
            if self.native_hierarchy
                && args.first().is_some_and(|mode| ["on", "off", "storage", "data"].iter().any(|word| mode.is_kw(word)))
            {
                return Err(KivaError::Denied("this connection stays in hierarchical mode".into()));
            }
            match args {
                [mode] if mode.is_kw("on") => {
                    self.hierarchy_access(&user, false)?;
                    let catalog = self.preview_catalog()?;
                    self.drop_subscriptions();
                    self.hierarchy_data = false;
                    self.hierarchy =
                        Some(Context::workspace(catalog.workspace("default").expect("validated default"), false));
                    out.say("Hierarchy preview enabled. Data operations are disabled.");
                }
                [mode] if mode.is_kw("storage") || mode.is_kw("data") => {
                    self.check_managed_access()?;
                    let inst = self.inst.clone();
                    Policy::load_cached(&inst)?;
                    let mut cache = inst
                        .managed_catalog
                        .lock()
                        .map_err(|_| KivaError::Other("managed catalog cache poisoned".into()))?;
                    let catalog = if let Some(catalog) = cache.as_ref() {
                        catalog.clone()
                    } else if let Some(catalog) = crate::managed_layout::open(&inst.home, &inst.engine_options())? {
                        Arc::new(catalog)
                    } else {
                        if user.role != Role::Admin {
                            return Err(hidden());
                        }
                        Arc::new(crate::managed_layout::edit(&inst.home, &inst.engine_options(), |_| Ok(()))?)
                    };
                    self.drop_subscriptions();
                    self.unwatch();
                    self.hierarchy =
                        Some(Context::workspace(catalog.workspace("default").expect("validated default"), true));
                    *cache = Some(catalog);
                    self.hierarchy_data = mode.is_kw("data");
                    out.say(if self.hierarchy_data {
                        "Local managed data mode enabled."
                    } else {
                        "Managed storage administration enabled. Data operations are disabled."
                    });
                }
                [mode] if mode.is_kw("permissions") => {
                    let mut context = self.hierarchy.ok_or_else(hidden)?;
                    let catalog = self.context_catalog(context.managed)?;
                    context.normalize(&catalog);
                    let access = self.hierarchy_access(&user, context.managed)?;
                    let scope = context.scope();
                    out.say(format!(
                        "read={} write={} manage={}",
                        access.allows(&catalog, scope, Permission::Read),
                        access.allows(&catalog, scope, Permission::Write),
                        access.allows(&catalog, scope, Permission::Manage)
                    ));
                }
                [mode] if mode.is_kw("engines") => {
                    if !self.hierarchy.is_some_and(|context| context.managed) || user.role != Role::Admin {
                        return Err(hidden());
                    }
                    let registry = self
                        .inst
                        .managed_engines
                        .lock()
                        .map_err(|_| KivaError::Other("managed engine registry poisoned".into()))?;
                    out.say(registry.summary());
                }
                [mode] if mode.is_kw("off") => {
                    self.hierarchy = None;
                    self.hierarchy_data = false;
                    out.say("Hierarchy preview disabled.");
                }
                [action, account, permission, kind, target @ ..] if action.is_kw("grant") || action.is_kw("revoke") => {
                    if !self.hierarchy.is_some_and(|context| context.managed) || user.role != Role::Admin {
                        return Err(hidden());
                    }
                    let catalog = self.context_catalog(true)?;
                    let scope = grant_scope(&catalog, kind, target)?;
                    let subject = auth::get_user(&self.inst.system, &account.text)?
                        .ok_or_else(|| KivaError::NotFound("account does not exist".into()))?;
                    Policy::edit(
                        &self.inst.system,
                        &subject,
                        scope,
                        Permission::parse(&permission.text.to_ascii_lowercase())?,
                        action.is_kw("grant"),
                    )?;
                    out.say("Hierarchy grants updated.");
                }
                _ => return Err(KivaError::Invalid("use hierarchy on, storage, data, off, grant or revoke".into())),
            }
            return Ok(());
        }
        let mut context = self.hierarchy.expect("preview dispatch");
        let catalog = self.context_catalog(context.managed)?;
        let access = self.hierarchy_access(&user, context.managed)?;
        context.normalize(&catalog);
        if matches!(command, "switch" | "use") {
            let next = navigate(&catalog, context, command, args).map_err(|error| {
                if access.role() == Role::Admin {
                    error
                } else {
                    hidden()
                }
            })?;
            if !access.visible(&catalog, next.scope()) {
                return Err(hidden());
            }
            out.say(format!("Using {}", next.path(&catalog, &access)));
            self.hierarchy = Some(next);
            return Ok(());
        }
        if matches!(command, "help" | "h") && args.is_empty() {
            out.say("switch <workspace> | use <project> | use <project>.<collection> | use collection <path>");
            out.say("workspace|project|collection create|list|current|info|exists|rename|drop|help");
            out.say("hierarchy off returns to the legacy session");
            return Ok(());
        }
        if command == "whoami" && args.is_empty() {
            out.say(format!("{} @ {} (hierarchy preview)", self.user_name(), context.path(&catalog, &access)));
            return Ok(());
        }
        let command = if command == "collections" { "collection" } else { command };
        if !matches!(command, "workspace" | "project" | "collection") {
            return Err(KivaError::Denied("only hierarchy metadata commands are available in preview".into()));
        }
        let (operation, args) =
            args.split_first().ok_or_else(|| KivaError::Invalid("missing hierarchy operation".into()))?;
        let op = operation.text.to_ascii_lowercase();
        if op == "help" && args.is_empty() {
            out.say(format!("{command} create <name> [...]"));
            let details = if command == "workspace" { " [--details]" } else { "" };
            out.say(format!("{command} list{details} | current | info <name> | exists <name>"));
            out.say(format!("{command} rename <old> to <new>"));
            let cascade = if command == "collection" { "" } else { " [cascade]" };
            out.say(format!("{command} drop <name>{cascade}"));
            let navigation = match command {
                "workspace" => "switch <workspace>",
                "project" => "use <project>",
                _ => "use collection <collection> | use <project>.<collection> | use collection <workspace>.<project>.<collection>",
            };
            out.say(navigation);
            return Ok(());
        }
        if op == "current" && args.is_empty() {
            let [workspace, project, collection] = context.visible_names(&catalog, &access);
            let name = match command {
                "workspace" => workspace,
                "project" => project,
                _ => collection,
            };
            out.say(if name.is_empty() { "none".to_owned() } else { name });
            return Ok(());
        }
        let parent = Parent::for_kind(command, context)?;
        let visible =
            |name: &str| parent.target(&catalog, name).filter(|target| access.visible(&catalog, target.scope()));
        let visible_names =
            || parent.names(&catalog).into_iter().filter(|name| visible(name).is_some()).collect::<Vec<_>>();
        let require_manage = |catalog: &Catalog, scope: Scope| {
            if access.allows(catalog, scope, Permission::Manage) {
                Ok(())
            } else {
                Err(hidden())
            }
        };
        match (op.as_str(), args) {
            ("list", []) => out.say(visible_names().join("\n")),
            ("list", [flag]) if command == "workspace" && flag.text == "--details" => {
                if self.native_hierarchy || access.role() != Role::Admin {
                    return Err(hidden());
                }
                out.say("WORKSPACE  PROJECTS  COLLECTIONS  PLANNED PATH");
                for name in visible_names() {
                    let id = catalog.workspace(name).expect("listed workspace");
                    let projects: Vec<_> = catalog
                        .project_names(id)
                        .filter(|name| {
                            access.visible(&catalog, Scope::Project(catalog.project(id, name).expect("listed project")))
                        })
                        .collect();
                    let collections: usize = projects
                        .iter()
                        .map(|name| {
                            let project = catalog.project(id, name).expect("listed project");
                            catalog
                                .collection_names(project)
                                .filter(|name| {
                                    access.visible(
                                        &catalog,
                                        Scope::Collection(
                                            catalog.collection(project, name).expect("listed collection"),
                                        ),
                                    )
                                })
                                .count()
                        })
                        .sum();
                    let path = crate::hierarchy_layout::workspace(&self.inst.home, name)?;
                    out.say(format!("{name}  {}  {collections}  {}", projects.len(), path.display()));
                }
            }
            ("exists", [name]) => out.say(visible(&name.text).is_some().to_string()),
            ("info", [name]) => {
                let target = visible(&name.text).ok_or_else(hidden)?;
                if access.role() == Role::Admin {
                    out.say(target.info(&catalog));
                } else {
                    out.say(name.text.clone());
                }
            }
            ("create", [_, ..]) => {
                self.edit_hierarchy(context.managed, |catalog| {
                    require_manage(catalog, parent.scope())?;
                    for name in args {
                        parent.create(catalog, &name.text)?;
                    }
                    Ok(())
                })?;
                out.say(format!("Created {} {command} entries", args.len()));
            }
            ("rename", [from, to] | [from, _, to]) if args.len() == 2 || args[1].is_kw("to") => {
                self.edit_hierarchy(context.managed, |catalog| {
                    let target = parent.target(catalog, &from.text).ok_or_else(hidden)?;
                    // Rename also writes into the parent's namespace. Requiring
                    // that scope prevents probing hidden siblings by collision.
                    require_manage(catalog, parent.scope())?;
                    target.rename(catalog, &to.text)
                })?;
                out.say(format!("Renamed '{}' to '{}'", from.text, to.text));
            }
            ("drop", [name] | [name, _]) if args.len() == 1 || args[1].is_kw("cascade") => {
                self.edit_hierarchy(context.managed, |catalog| {
                    let target = parent.target(catalog, &name.text).ok_or_else(hidden)?;
                    require_manage(catalog, target.scope())?;
                    target.drop_from(catalog, args.len() == 2)
                })?;
                out.say(format!("Dropped '{}'", name.text));
            }
            _ => return Err(KivaError::Invalid(format!("invalid {command} {op} command"))),
        }
        self.hierarchy = Some(context);
        Ok(())
    }

    pub(crate) fn enter_native_hierarchy(&mut self) -> Result<()> {
        let inst = self.inst.clone();
        let _guard = lock_layout(&inst, true, false)?;
        let catalog = self.context_catalog(true)?;
        self.hierarchy = Some(Context::workspace(catalog.workspace("default").expect("validated default"), true));
        self.hierarchy_data = true;
        Ok(())
    }

    fn check_managed_access(&self) -> Result<()> {
        self.check_managed_session()?;
        self.inst.check_managed_layout_home()
    }

    fn check_managed_session(&self) -> Result<()> {
        let admitted = if self.native_hierarchy {
            self.inst.hierarchical_tcp && self.hierarchy_subject.is_some()
        } else {
            self.local
                && !self.inst.hierarchical_tcp
                && self.inst.tcp_port.load(std::sync::atomic::Ordering::Acquire) == 0
        };
        if !admitted
            || self.inst.shutting_down.load(std::sync::atomic::Ordering::Acquire)
            || self.inst.repl.is_replica()
        {
            let message = if self.native_hierarchy {
                "managed native session is unavailable"
            } else {
                "managed storage requires the local shell with --no-listen"
            };
            return Err(KivaError::Denied(message.into()));
        }
        Ok(())
    }

    fn context_catalog(&self, managed: bool) -> Result<Arc<Catalog>> {
        if !managed {
            return self.preview_catalog().map(Arc::new);
        }
        self.check_managed_session()?;
        let mut cache =
            self.inst.managed_catalog.lock().map_err(|_| KivaError::Other("managed catalog cache poisoned".into()))?;
        if let Some(catalog) = &*cache {
            return Ok(catalog.clone());
        }
        self.inst.check_managed_layout_home().map_err(|e| self.managed_storage_error(e))?;
        let catalog = crate::managed_layout::open(&self.inst.home, &self.inst.engine_options())
            .map_err(|e| self.managed_storage_error(e))?
            .ok_or_else(|| KivaError::Corrupted("missing managed layout".into()))?;
        let catalog = Arc::new(catalog);
        *cache = Some(catalog.clone());
        Ok(catalog)
    }

    /// Grants, engine lookup, and execution are linearized by the same lock
    /// used by revocation and layout changes. The legacy engine is restored
    /// before returning. An open transaction retains its exact engine.
    pub(crate) fn with_managed_engine<T>(
        &mut self,
        command: &str,
        need: Need,
        execute: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let inst = self.inst.clone();
        let _guard = lock_layout(&inst, true, command == "compact")?;
        if command == "rollback" && need == Need::None {
            return execute(self);
        }
        let route = (|| -> Result<_> {
            self.check_managed_session()?;
            let context = self.hierarchy.filter(|context| context.managed).ok_or_else(hidden)?;
            let mut selected = context;
            let id = context
                .collection
                .ok_or_else(|| KivaError::Invalid("No collection selected. Use 'use collection <name>'.".into()))?;
            let catalog = self.context_catalog(true)?;
            selected.normalize(&catalog);
            if selected.collection != Some(id) {
                return Err(hidden());
            }
            let user = self.hierarchy_user()?;
            let access = self.hierarchy_access(&user, true)?;
            let scope = Scope::Collection(id);
            let read = access.allows(&catalog, scope, Permission::Read);
            let write = access.allows(&catalog, scope, Permission::Write);
            let manage = access.allows(&catalog, scope, Permission::Manage);
            if !(read && (need != Need::Write || write) && (need != Need::Owner || write && manage)) {
                return Err(KivaError::Denied("insufficient permission on managed collection".into()));
            }
            if need == Need::Owner && self.txn.is_some() {
                return Err(KivaError::Invalid("collection maintenance is blocked during a transaction".into()));
            }
            let mut engines =
                inst.managed_engines.lock().map_err(|_| KivaError::Other("managed engine registry poisoned".into()))?;
            if command == "compact" && engines.pinned(id) {
                return Err(KivaError::Invalid("compaction is blocked during a collection transaction".into()));
            }
            let engine = engines.get_or_open(id, || {
                crate::managed_layout::open_collection(&inst.home, &catalog, id, inst.engine_options())
                    .map_err(|e| self.managed_storage_error(e))
            })?;
            if self
                .managed_txn_engine
                .as_ref()
                .is_some_and(|(pinned_id, pinned)| *pinned_id != id || !std::sync::Arc::ptr_eq(pinned, &engine))
            {
                return Err(KivaError::Conflict("managed transaction engine changed".into()));
            }
            Ok((id, engine, write, manage, selected.names(&catalog).join(".")))
        })();
        let (id, engine, write, manage, label) = match route {
            Ok(route) => route,
            Err(error) => {
                if command == "commit" {
                    self.rollback();
                }
                return Err(error);
            }
        };
        let old_engine = std::mem::replace(&mut self.engine, engine.clone());
        let old_label = std::mem::replace(&mut self.ws, label);
        self.managed_dispatch = Some((write, manage));
        let result = execute(self);
        self.managed_dispatch = None;
        self.engine = old_engine;
        self.ws = old_label;
        if command == "compact" && result.is_err() {
            // Force validation and intent recovery before admitting another
            // command after an uncertain replacement or publication failure.
            inst.managed_engines
                .lock()
                .map_err(|_| KivaError::Other("managed engine registry poisoned".into()))?
                .discard(id);
        }
        if self.txn.is_some() && self.managed_txn_engine.is_none() {
            self.managed_txn_engine = Some((id, engine));
        }
        // Managed engines are not hooked into legacy replication.
        self.wrote = false;
        result
    }

    pub(crate) fn compact_managed(&self) -> Result<crate::engine::CompactReport> {
        let id = self.hierarchy.and_then(|context| context.collection).ok_or_else(hidden)?;
        let catalog = self.context_catalog(true)?;
        crate::managed_layout::compact_collection(&self.inst.home, &catalog, id, &self.engine)
            .map_err(|e| self.managed_storage_error(e))
    }

    fn managed_storage_error(&self, error: KivaError) -> KivaError {
        if self.native_hierarchy {
            KivaError::Other("managed storage operation failed".into())
        } else {
            error
        }
    }

    fn preview_catalog(&self) -> Result<Catalog> {
        match self.inst.workspace_catalog()? {
            Some(catalog) => Ok(catalog),
            None => Ok(CatalogStore::preview(&self.inst.system).load()?.unwrap_or_default()),
        }
    }

    fn edit_hierarchy(&self, managed: bool, edit: impl FnOnce(&mut Catalog) -> Result<()>) -> Result<()> {
        if managed {
            self.check_managed_access()?;
            let mut engines = self
                .inst
                .managed_engines
                .lock()
                .map_err(|_| KivaError::Other("managed engine registry poisoned".into()))?;
            if engines.any_pinned() {
                return Err(KivaError::Invalid(
                    "a managed transaction is active; finish it before a layout change".into(),
                ));
            }
            engines.clear();
            drop(engines);
            let mut cache = self
                .inst
                .managed_catalog
                .lock()
                .map_err(|_| KivaError::Other("managed catalog cache poisoned".into()))?;
            cache.take();
            let catalog = crate::managed_layout::edit(&self.inst.home, &self.inst.engine_options(), edit)
                .map_err(|e| self.managed_storage_error(e))?;
            *cache = Some(Arc::new(catalog));
            return Ok(());
        }
        if self.inst.workspace_catalog()?.is_some() || self.inst.repl.is_replica() {
            return Err(KivaError::ReadOnly("a staged migration or replica catalog is read-only".into()));
        }
        CatalogStore::preview(&self.inst.system).edit_preview(edit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    fn run(session: &mut Session, command: &str) {
        let result = session.execute(command);
        assert!(!result.has_err(), "{command}: {:?}", result.lines);
    }

    #[test]
    #[ignore = "requires KIVADB_HIERARCHY_BENCH_HOME provisioned by the Release example"]
    fn managed_routing_component_costs() {
        use std::hint::black_box;
        use std::time::Instant;

        let Some(path) = std::env::var_os("KIVADB_HIERARCHY_BENCH_HOME") else { return };
        let home = std::path::Path::new(&path);
        let inst = crate::Instance::open(home, crate::Options::default()).unwrap();
        let mut session = Session::new(inst.clone(), true, None);
        session.login("editor0", "bench-test").unwrap();
        run(&mut session, "hierarchy data");
        run(&mut session, "use collection w0.p0.c0");
        let catalog = session.context_catalog(true).unwrap();
        let target = session.hierarchy.unwrap().scope();
        let user = session.hierarchy_user().unwrap();
        let access = session.hierarchy_access(&user, true).unwrap();

        fn measure(label: &str, samples: usize, mut operation: impl FnMut()) {
            let mut values = (0..samples)
                .map(|_| {
                    let start = Instant::now();
                    operation();
                    start.elapsed().as_nanos()
                })
                .collect::<Vec<_>>();
            values.sort_unstable();
            let percentile = |rank: usize| values[(samples * rank).div_ceil(100) - 1];
            println!("{label}: n={samples} p50={} p95={} p99={} ns", percentile(50), percentile(95), percentile(99));
        }

        measure("validated catalog load", 16, || {
            black_box(crate::managed_layout::open(home, &inst.engine_options()).unwrap());
        });
        measure("policy load and index", 128, || {
            black_box(Policy::load(&inst.system).unwrap());
        });
        measure("catalog Arc lookup", 2048, || {
            black_box(session.context_catalog(true).unwrap());
        });
        measure("collection ID and parent resolution", 2048, || {
            black_box(catalog.resolve_collection("w0", "p0", "c0").unwrap());
        });
        measure("current account validation", 2048, || {
            black_box(session.hierarchy_user().unwrap());
        });
        measure("versioned policy and account lookup", 2048, || {
            black_box(session.hierarchy_access(&user, true).unwrap());
        });
        measure("inherited permission", 2048, || {
            assert!(black_box(access.allows(&catalog, target, Permission::Write)));
        });
        measure("uncontended layout guard", 2048, || {
            drop(black_box(lock_layout(&inst, true, false).unwrap()));
        });
        drop(session);
        inst.shutdown();
    }

    #[test]
    fn active_data_commands_share_admission_but_exclude_layout_and_grant_writers() {
        let path =
            std::env::temp_dir().join(format!("kiva-managed-concurrency-{:016x}", crate::util::random_u64().unwrap()));
        let inst = crate::Instance::open(&path, crate::Options::default()).unwrap();
        let mut held = Session::new(inst.clone(), true, None);
        for command in [
            "hierarchy data",
            "workspace create first second",
            "switch first",
            "project create app",
            "use app",
            "collection create data",
            "switch second",
            "project create app",
            "use app",
            "collection create data",
            "use collection first.app.data",
            "set first 1",
        ] {
            run(&mut held, command);
        }
        let mut same = Session::new(inst.clone(), true, None);
        let mut other = Session::new(inst.clone(), true, None);
        for (session, collection) in [(&mut same, "first.app.data"), (&mut other, "second.app.data")] {
            run(session, "hierarchy data");
            run(session, &format!("use collection {collection}"));
        }
        std::thread::scope(|scope| {
            let (active, admitted) = mpsc::channel();
            let (release, wait) = mpsc::channel();
            let handle = scope.spawn(move || {
                held.with_managed_engine("get", Need::Read, |_| {
                    active.send(()).unwrap();
                    wait.recv_timeout(Duration::from_secs(10)).unwrap();
                    Ok(())
                })
                .unwrap();
            });
            admitted.recv_timeout(Duration::from_secs(10)).unwrap();
            assert!(matches!(inst.managed_layout_lock.try_write(), Err(std::sync::TryLockError::WouldBlock)));
            let (done, completed) = mpsc::channel();
            scope.spawn(move || {
                run(&mut same, "get first");
                run(&mut other, "begin");
                run(&mut other, "set second 2");
                run(&mut other, "commit");
                done.send(()).unwrap();
            });
            let completed_while_held = completed.recv_timeout(Duration::from_secs(5));
            release.send(()).unwrap();
            handle.join().unwrap();
            completed_while_held.expect("active operation must admit commands in both stores");
        });
        inst.shutdown();
        drop(inst);
        let _ = std::fs::remove_dir_all(path);
    }
}
