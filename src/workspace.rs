//! Workspace persistence — save/load workspace count and names.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Serialize, Deserialize)]
struct WorkspaceState {
    count: u8,
    names: Vec<String>,
}

fn workspaces_path() -> Result<PathBuf> {
    let base = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("bobnote");
    fs::create_dir_all(&base)?;
    Ok(base.join("workspaces.json"))
}

pub fn save_workspaces(count: u8, names: &[String]) -> Result<()> {
    let path = workspaces_path()?;
    let json = serde_json::to_string(&WorkspaceState {
        count,
        names: names.to_vec(),
    })?;
    fs::write(path, json)?;
    Ok(())
}

/// Returns (count, names). Always has at least 1 workspace.
pub fn load_workspaces() -> (u8, Vec<String>) {
    let path = match workspaces_path() {
        Ok(p) => p,
        Err(_) => return (1, vec!["WS 1".to_string()]),
    };
    let json = match fs::read_to_string(path) {
        Ok(j) => j,
        Err(_) => return (1, vec!["WS 1".to_string()]),
    };
    match serde_json::from_str::<WorkspaceState>(&json) {
        Ok(s) => {
            let count = s.count.max(1);
            let mut names = s.names;
            while names.len() < count as usize {
                names.push(format!("WS {}", names.len() + 1));
            }
            (count, names)
        }
        Err(_) => (1, vec!["WS 1".to_string()]),
    }
}
