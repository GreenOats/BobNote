//! Notebook persistence — named collections of notes (book-style groups).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{collections::{HashMap, HashSet}, fs, path::PathBuf};

/// A named, ordered collection of note IDs.
/// Stored in `~/.local/share/bobnote/notebooks/<id>.json`.
#[derive(Serialize, Deserialize, Clone)]
pub struct NotebookData {
    pub id: u64,
    pub title: String,
    /// Ordered list of note IDs that make up the notebook's pages.
    pub note_ids: Vec<u64>,
    /// When true, open book pages float across all workspaces (legacy behaviour).
    /// When false (default), pages are filtered to their own workspace like regular notes.
    #[serde(default)]
    pub persistent: bool,
}

fn notebooks_dir() -> Result<PathBuf> {
    let base = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("bobnote")
        .join("notebooks");
    fs::create_dir_all(&base)?;
    Ok(base)
}

pub fn save_notebooks(notebooks: &[NotebookData]) -> Result<()> {
    let dir = notebooks_dir()?;

    // Delete files for notebooks that no longer exist.
    let active_ids: HashSet<u64> = notebooks.iter().map(|nb| nb.id).collect();
    for entry in fs::read_dir(&dir)?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            if let Some(id) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<u64>().ok())
            {
                if !active_ids.contains(&id) {
                    let _ = fs::remove_file(&path);
                }
            }
        }
    }

    for nb in notebooks {
        let path = dir.join(format!("{}.json", nb.id));
        let json = serde_json::to_string_pretty(nb)?;
        fs::write(path, json)?;
    }
    Ok(())
}

pub fn load_notebooks() -> Result<Vec<NotebookData>> {
    let dir = notebooks_dir()?;
    let mut notebooks = Vec::new();

    for entry in fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            // Skip the session file — it's not a NotebookData.
            if path.file_stem().and_then(|s| s.to_str()) == Some("_session") {
                continue;
            }
            let json = fs::read_to_string(&path)?;
            if let Ok(data) = serde_json::from_str::<NotebookData>(&json) {
                notebooks.push(data);
            }
        }
    }

    notebooks.sort_by_key(|nb| nb.id);
    Ok(notebooks)
}

// ---------------------------------------------------------------------------
// Session state — persists which notebook (if any) was open in book mode
// ---------------------------------------------------------------------------

/// All notebooks open in book mode when the app last exited.
/// New format stores a list of (nb_id, page_idx) pairs.
#[derive(Serialize, Deserialize)]
pub struct NotebookSession {
    pub open: Vec<(u64, usize)>,
}

fn session_path() -> Result<PathBuf> {
    Ok(notebooks_dir()?.join("_session.json"))
}

/// Save all currently-open notebooks to disk.
/// An empty map removes the session file so no books reopen on next launch.
pub fn save_session(open: &HashMap<u64, usize>) -> Result<()> {
    let path = session_path()?;
    if open.is_empty() {
        let _ = fs::remove_file(path);
    } else {
        let entries: Vec<(u64, usize)> = open.iter().map(|(&k, &v)| (k, v)).collect();
        let json = serde_json::to_string(&NotebookSession { open: entries })?;
        fs::write(path, json)?;
    }
    Ok(())
}

/// Load the session from disk.
/// Returns a list of (nb_id, page_idx) pairs for all notebooks that were open.
/// Handles the old single-notebook format for backward compatibility.
pub fn load_session() -> Vec<(u64, usize)> {
    let path = match session_path() {
        Ok(p) => p,
        Err(_) => return vec![],
    };
    let json = match fs::read_to_string(path) {
        Ok(j) => j,
        Err(_) => return vec![],
    };
    // Try new multi-notebook format first.
    if let Ok(s) = serde_json::from_str::<NotebookSession>(&json) {
        return s.open;
    }
    // Backward compat: old format had a single notebook_id + page_idx.
    #[derive(Deserialize)]
    struct OldSession { notebook_id: u64, page_idx: usize }
    if let Ok(s) = serde_json::from_str::<OldSession>(&json) {
        return vec![(s.notebook_id, s.page_idx)];
    }
    vec![]
}
