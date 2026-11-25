// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Paths for managed hierarchy resources. Building a path does not create it.

use crate::error::{KivaError, Result};
use std::path::{Path, PathBuf};

pub(crate) const EXPORT_MARKER: &str = "workspace-layout-export.json";

pub(crate) fn segment(name: &str) -> Result<&str> {
    let upper = name.to_ascii_uppercase();
    let device = matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "SYSTEM")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit());
    if device || !crate::auth::valid_name_part(name) {
        return Err(KivaError::Invalid(format!("invalid managed hierarchy name '{name}'")));
    }
    Ok(name)
}

pub(crate) fn workspace(home: &Path, name: &str) -> Result<PathBuf> {
    Ok(home.join("workspaces").join(segment(name)?))
}

pub(crate) fn project(home: &Path, workspace_name: &str, project_name: &str) -> Result<PathBuf> {
    Ok(workspace(home, workspace_name)?.join("projects").join(segment(project_name)?))
}

pub(crate) fn collection(home: &Path, workspace_name: &str, project_name: &str, name: &str) -> Result<PathBuf> {
    Ok(project(home, workspace_name, project_name)?.join("collections").join(segment(name)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_segments_reject_paths_and_reserved_devices_on_all_platforms() {
        for invalid in ["", ".", "..", "../other", "/absolute", "c:\\absolute", "a/b", "nul", "LPT1", "system"] {
            assert!(workspace(Path::new("home"), invalid).is_err(), "{invalid}");
        }
        assert_eq!(
            collection(Path::new("home"), "Trading", "MaplaTrade", "agents").unwrap(),
            Path::new("home/workspaces/Trading/projects/MaplaTrade/collections/agents")
        );
    }
}
