// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Administrative metadata preview. Collection identities are never passed to
//! the historical storage engine or its authorization and replication paths.

use crate::auth::{self, Role};
use crate::catalog::{Catalog, CatalogStore, CollectionId, ProjectId, WorkspaceId};
use crate::error::{KivaError, Result};
use crate::exec::Out;
use crate::lexer::Token;
use crate::session::Session;

#[derive(Clone, Copy)]
pub(crate) struct Context {
    workspace: WorkspaceId,
    project: Option<ProjectId>,
    collection: Option<CollectionId>,
}

impl Context {
    fn workspace(workspace: WorkspaceId) -> Self {
        Self { workspace, project: None, collection: None }
    }

    fn normalize(&mut self, catalog: &Catalog) {
        if catalog.workspace_name(self.workspace).is_none() {
            *self = Self::workspace(catalog.workspace("default").expect("validated default workspace"));
        } else if self.project.is_some_and(|id| catalog.project_name(id).is_none()) {
            *self = Self::workspace(self.workspace);
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

    fn path(self, catalog: &Catalog) -> String {
        self.names(catalog).into_iter().filter(|name| !name.is_empty()).collect::<Vec<_>>().join(".")
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
        match self {
            Self::Root => catalog.create_workspace(name).map(|_| ()),
            Self::Workspace(parent) => catalog.create_project(parent, name).map(|_| ()),
            Self::Project(parent) => catalog.create_collection(parent, name).map(|_| ()),
        }
    }
}

impl Target {
    fn rename(self, catalog: &mut Catalog, name: &str) -> Result<()> {
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

fn lookup<T>(found: Option<T>, kind: &str, name: &str) -> Result<T> {
    found.ok_or_else(|| KivaError::NotFound(format!("{kind} '{name}' does not exist")))
}

fn navigate(catalog: &Catalog, mut current: Context, command: &str, args: &[Token]) -> Result<Context> {
    match (command, args) {
        ("switch", [name]) => {
            return Ok(Context::workspace(lookup(catalog.workspace(&name.text), "workspace", &name.text)?))
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
    Ok(Context { workspace, project: Some(project), collection: Some(id) })
}

impl Session {
    fn check_hierarchy_admin(&self) -> Result<()> {
        let user =
            self.user.as_ref().ok_or_else(|| KivaError::Denied("hierarchy preview requires authentication".into()))?;
        if (!self.local && !self.inst.auth_required())
            || user.role != Role::Admin
            || !auth::get_user(&self.inst.system, &user.name)?.is_some_and(|account| account.role == Role::Admin)
        {
            return Err(KivaError::Denied("hierarchy preview requires an administrator and TCP authentication".into()));
        }
        Ok(())
    }

    pub(crate) fn hierarchy_location(&self) -> Result<Option<[String; 3]>> {
        let Some(mut context) = self.hierarchy else { return Ok(None) };
        self.check_hierarchy_admin()?;
        let catalog = self.preview_catalog()?;
        context.normalize(&catalog);
        Ok(Some(context.names(&catalog).map(str::to_owned)))
    }

    pub(crate) fn execute_hierarchy(
        &mut self,
        command: &str,
        args: &[Token],
        out: &mut Out,
    ) -> std::result::Result<(), String> {
        self.hierarchy_command(command, args, out).map_err(|error| error.to_string())
    }

    fn hierarchy_command(&mut self, command: &str, args: &[Token], out: &mut Out) -> Result<()> {
        self.check_hierarchy_admin()?;
        if self.txn.is_some() {
            return Err(KivaError::Invalid("hierarchy preview is unavailable during a transaction".into()));
        }
        if command == "hierarchy" {
            match args {
                [mode] if mode.is_kw("on") => {
                    let catalog = self.preview_catalog()?;
                    self.drop_subscriptions();
                    self.hierarchy = Some(Context::workspace(catalog.workspace("default").expect("validated default")));
                    out.say("Hierarchy preview enabled. Data operations are disabled.");
                }
                [mode] if mode.is_kw("off") => {
                    self.hierarchy = None;
                    out.say("Hierarchy preview disabled.");
                }
                _ => return Err(KivaError::Invalid("use hierarchy on or hierarchy off".into())),
            }
            return Ok(());
        }
        let mut context = self.hierarchy.expect("preview dispatch");
        let catalog = self.preview_catalog()?;
        context.normalize(&catalog);
        if matches!(command, "switch" | "use") {
            let next = navigate(&catalog, context, command, args)?;
            out.say(format!("Using {}", next.path(&catalog)));
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
            out.say(format!("{} @ {} (hierarchy preview)", self.user_name(), context.path(&catalog)));
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
            let [workspace, project, collection] = context.names(&catalog);
            let name = match command {
                "workspace" => workspace,
                "project" => project,
                _ => collection,
            };
            out.say(if name.is_empty() { "none" } else { name });
            return Ok(());
        }
        let parent = Parent::for_kind(command, context)?;
        match (op.as_str(), args) {
            ("list", []) => out.say(parent.names(&catalog).join("\n")),
            ("list", [flag]) if command == "workspace" && flag.text == "--details" => {
                out.say("WORKSPACE  PROJECTS  COLLECTIONS  PLANNED PATH");
                for name in parent.names(&catalog) {
                    let id = catalog.workspace(name).expect("listed workspace");
                    let projects: Vec<_> = catalog.project_names(id).collect();
                    let collections: usize = projects
                        .iter()
                        .map(|name| {
                            let project = catalog.project(id, name).expect("listed project");
                            catalog.collection_names(project).count()
                        })
                        .sum();
                    let path = crate::hierarchy_layout::workspace(&self.inst.home, name)?;
                    out.say(format!("{name}  {}  {collections}  {}", projects.len(), path.display()));
                }
            }
            ("exists", [name]) => out.say(parent.target(&catalog, &name.text).is_some().to_string()),
            ("info", [name]) => {
                out.say(lookup(parent.target(&catalog, &name.text), command, &name.text)?.info(&catalog))
            }
            ("create", [_, ..]) => {
                self.edit_hierarchy(|catalog| {
                    for name in args {
                        parent.create(catalog, &name.text)?;
                    }
                    Ok(())
                })?;
                out.say(format!("Created {} {command} entries", args.len()));
            }
            ("rename", [from, to] | [from, _, to]) if args.len() == 2 || args[1].is_kw("to") => {
                self.edit_hierarchy(|catalog| {
                    lookup(parent.target(catalog, &from.text), command, &from.text)?.rename(catalog, &to.text)
                })?;
                out.say(format!("Renamed '{}' to '{}'", from.text, to.text));
            }
            ("drop", [name] | [name, _]) if args.len() == 1 || args[1].is_kw("cascade") => {
                self.edit_hierarchy(|catalog| {
                    lookup(parent.target(catalog, &name.text), command, &name.text)?.drop_from(catalog, args.len() == 2)
                })?;
                out.say(format!("Dropped '{}'", name.text));
            }
            _ => return Err(KivaError::Invalid(format!("invalid {command} {op} command"))),
        }
        self.hierarchy = Some(context);
        Ok(())
    }

    fn preview_catalog(&self) -> Result<Catalog> {
        match self.inst.workspace_catalog()? {
            Some(catalog) => Ok(catalog),
            None => Ok(CatalogStore::preview(&self.inst.system).load()?.unwrap_or_default()),
        }
    }

    fn edit_hierarchy(&self, edit: impl FnOnce(&mut Catalog) -> Result<()>) -> Result<()> {
        if self.inst.workspace_catalog()?.is_some() || self.inst.repl.is_replica() {
            return Err(KivaError::ReadOnly("a staged migration or replica catalog is read-only".into()));
        }
        CatalogStore::preview(&self.inst.system).edit_preview(edit)
    }
}
