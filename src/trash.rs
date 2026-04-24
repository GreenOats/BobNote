//! Recycle-bin persistence — trashed notes live in ~/.local/share/bobnote/trash/.

use crate::note::NoteData;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, time::{SystemTime, UNIX_EPOCH}};

#[derive(Serialize, Deserialize, Clone)]
pub struct TrashedNote {
    #[serde(flatten)]
    pub data: NoteData,
    /// Unix timestamp (seconds) when the note was trashed.
    pub deleted_at: u64,
}

fn trash_dir() -> Result<PathBuf> {
    let base = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("bobnote")
        .join("trash");
    fs::create_dir_all(&base)?;
    Ok(base)
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Write a single trashed note to the trash dir immediately.
pub fn save_trash_note(note: &TrashedNote) -> Result<()> {
    let path = trash_dir()?.join(format!("{}.json", note.data.id));
    fs::write(path, serde_json::to_string_pretty(note)?)?;
    Ok(())
}

/// Delete a single trashed note's file.
pub fn delete_trash_note(id: u64) {
    if let Ok(dir) = trash_dir() {
        let _ = fs::remove_file(dir.join(format!("{}.json", id)));
    }
}

/// Delete every file in the trash dir.
pub fn clear_trash_dir() {
    if let Ok(dir) = trash_dir() {
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("json") {
                    let _ = fs::remove_file(p);
                }
            }
        }
    }
}

/// Load all trashed notes from disk, sorted oldest-deleted first.
pub fn load_trash() -> Vec<TrashedNote> {
    let dir = match trash_dir() {
        Ok(d) => d,
        Err(_) => return vec![],
    };
    let mut notes = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Ok(json) = fs::read_to_string(&p) {
                    if let Ok(n) = serde_json::from_str::<TrashedNote>(&json) {
                        notes.push(n);
                    }
                }
            }
        }
    }
    notes.sort_by_key(|n| n.deleted_at);
    notes
}

/// Format a unix timestamp as a short relative age string ("2m", "3h", "4d", …).
pub fn format_age(deleted_at: u64) -> String {
    let now = now_secs();
    let secs = now.saturating_sub(deleted_at);
    if secs < 60 { return format!("{}s", secs); }
    let mins = secs / 60;
    if mins < 60 { return format!("{}m", mins); }
    let hours = mins / 60;
    if hours < 24 { return format!("{}h", hours); }
    let days = hours / 24;
    format!("{}d", days)
}
