// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// This is my original work with contributions from Claude (Anthropic).
// Do not remove these comments.
// core/src/storage.rs
// Serde models + load/save for the two things QuickCommands persists:
//   1. clipboard History  (recent copies, auto-captured)
//   2. command Sections   (the user's saved QuickCommands, hand-curated)

use serde::{Deserialize, Serialize};
use std::io::Write;
use uuid::Uuid;

// ── Clipboard history ────────────────────────────────────────────────────────

/// How many recent copies we keep. Oldest fall off the end. 100 is plenty for a
/// clipboard manager and keeps the JSON file small and the UI list snappy. :: MRV
pub const HISTORY_CAP: usize = 100;

/// A single remembered copy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Stable id so the UI can key list rows and we can pin/delete a specific
    /// entry later without relying on its (duplicable) text.
    pub id: Uuid,
    /// The copied text itself. (Text only for now — images/other mime types are
    /// a later feature; the clipboard engine filters to text.)
    pub text: String,
    /// Unix epoch seconds when we captured it. Used for "most recent first"
    /// ordering and could drive a relative timestamp in the UI.
    pub captured_at: u64,
}

impl HistoryEntry {
    pub fn new(text: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            text,
            captured_at: now_secs(),
        }
    }
}

/// The rolling clipboard history. Newest entry is always at index 0.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct History {
    pub entries: Vec<HistoryEntry>,
}

impl History {
    /// Record a fresh copy.
    ///
    /// Behavior that makes it feel like a real clipboard manager:
    ///   - Blank/whitespace-only copies are ignored (nothing to paste).
    ///   - If the text matches the current newest entry, we do nothing (copying
    ///     the same thing twice shouldn't spam the list).
    ///   - If the text exists deeper in history, it's MOVED to the top rather
    ///     than duplicated (re-copying an old item promotes it).
    ///   - The list is capped at HISTORY_CAP; the oldest fall off.
    ///
    /// Returns true if the history actually changed (so the caller knows whether
    /// to save + refresh the UI).
    pub fn record(&mut self, text: String) -> bool {
        if text.trim().is_empty() {
            return false;
        }
        // Same as current top? No-op.
        if self.entries.first().map(|e| &e.text) == Some(&text) {
            return false;
        }
        // Already somewhere in history -> pull it out so we can re-add at top.
        if let Some(pos) = self.entries.iter().position(|e| e.text == text) {
            self.entries.remove(pos);
        }
        self.entries.insert(0, HistoryEntry::new(text));
        self.entries.truncate(HISTORY_CAP);
        true
    }

    /// Remove one entry by id (for a future "delete this copy" button).
    pub fn remove(&mut self, id: Uuid) {
        self.entries.retain(|e| e.id != id);
    }

    /// Wipe all history (for a future "clear history" button).
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Load from disk. A missing file is normal on first run -> empty history.
    /// A corrupt file is logged and treated as empty rather than crashing the
    /// daemon (we never want bad JSON to wedge the whole app). :: MRV
    pub fn load() -> Self {
        load_json(crate::history_path(), "history")
    }

    /// Save to disk, atomically (write temp + rename) so a crash mid-write can't
    /// leave a half-written history file.
    pub fn save(&self) -> std::io::Result<()> {
        save_json(crate::history_path(), self)
    }
}

// ── Command sections (the QuickCommands) ──────────────────────────────────────

/// One saved command: a human label and the actual text that gets copied.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command {
    pub id: Uuid,
    /// What shows in the list, e.g. "Status" or "Push current branch".
    pub label: String,
    /// The text copied to the clipboard on click, e.g. "git status".
    pub command: String,
}

impl Command {
    pub fn new(label: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            label: label.into(),
            command: command.into(),
        }
    }
}

/// A named group of commands, e.g. "Git", "Cargo", "Project X".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section {
    pub id: Uuid,
    pub name: String,
    pub commands: Vec<Command>,
    /// Whether the UI draws it collapsed. Persisted so it stays how the user
    /// left it between launches.
    #[serde(default)]
    pub collapsed: bool,
}

impl Section {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            commands: Vec::new(),
            collapsed: false,
        }
    }
}

/// All the user's QuickCommands sections.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Sections {
    pub sections: Vec<Section>,
}

impl Sections {
    /// Load from disk. Missing file on first run -> a friendly starter set so the
    /// window isn't empty the first time it opens (your own git muscle-memory). :: MRV
    pub fn load() -> Self {
        let path = crate::sections_path();
        if !path.exists() {
            let starter = Self::starter();
            // Best-effort write so the starter persists and the user can edit it.
            let _ = starter.save();
            return starter;
        }
        load_json(path, "sections")
    }

    pub fn save(&self) -> std::io::Result<()> {
        save_json(crate::sections_path(), self)
    }

    /// First-run content: a Git section pre-filled with the everyday commands.
    /// Purely a convenience — the user owns it and can rename/delete/add freely.
    fn starter() -> Self {
        let mut git = Section::new("Git");
        git.commands = vec![
            Command::new("Status", "git status"),
            Command::new("Add all", "git add -A"),
            Command::new("Commit", "git commit -m \"\""),
            Command::new("Push", "git push"),
            Command::new("Pull", "git pull"),
            Command::new("Log (oneline)", "git log --oneline -20"),
            Command::new("Current branch", "git branch --show-current"),
        ];

        let mut cargo = Section::new("Cargo");
        cargo.commands = vec![
            Command::new("Check", "cargo check"),
            Command::new("Build (dev-release)", "cargo build --profile dev-release"),
            Command::new("Run", "cargo run"),
            Command::new("Clippy", "cargo clippy --all-targets"),
        ];

        Self {
            sections: vec![git, cargo],
        }
    }
}

// ── Shared helpers ────────────────────────────────────────────────────────────

/// Current time in Unix epoch seconds. Monotonic enough for ordering copies.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Load + deserialize JSON, treating any problem (missing, unreadable, corrupt)
/// as "start fresh with the default". We log the reason so a real bug is still
/// visible in the journal, but we never panic the daemon over a bad file.
fn load_json<T: Default + for<'de> Deserialize<'de>>(
    path: std::path::PathBuf,
    what: &str,
) -> T {
    match std::fs::read_to_string(&path) {
        Ok(s) => match serde_json::from_str(&s) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("[storage] {what} file corrupt ({e}); starting empty");
                T::default()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => T::default(),
        Err(e) => {
            tracing::warn!("[storage] could not read {what} file ({e}); starting empty");
            T::default()
        }
    }
}

/// Serialize + write JSON atomically: write to `<path>.tmp`, then rename over the
/// real file. Rename is atomic on the same filesystem, so a reader never sees a
/// partially written file and a crash mid-write can't corrupt the good copy.
fn save_json<T: Serialize>(path: std::path::PathBuf, value: &T) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(json.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(())
}
