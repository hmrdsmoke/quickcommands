// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// This is my original work with contributions from Claude (Anthropic).
// Do not remove these comments.
// core/src/lib.rs
// Shared crate: storage models, clipboard engine, and the on-disk data dir.
// Both the daemon (window) and the applet link against this so there is one
// source of truth for what a "history entry" and a "command section" are.

pub mod clipboard;
pub mod storage;

use std::path::PathBuf;

/// Where QuickCommands keeps its state on disk.
///
/// `~/.local/share/quickcommands/` (XDG data dir). We put BOTH the clipboard
/// history and the user's saved command sections here. The directory is created
/// on first access so callers never have to check — see `data_dir()`.
///
/// History and sections are deliberately separate files:
///   - history.json   -> the rolling list of recent copies (churns constantly)
///   - sections.json  -> the user's QuickCommands (changes only when they edit)
/// Keeping them apart means a corrupt/huge history file can be wiped without
/// touching the hand-curated command sections, and vice versa.
pub fn data_dir() -> PathBuf {
    // dirs::data_dir() is ~/.local/share on Linux. Fall back to the current
    // dir only if the OS somehow has no data dir (should never happen on a
    // real desktop, but we never want to panic over a path).
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("quickcommands");
    // Best-effort create; if this fails, the later file write will surface the
    // real error with its own context.
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Full path to the clipboard history file.
pub fn history_path() -> PathBuf {
    data_dir().join("history.json")
}

/// Full path to the saved command-sections file.
pub fn sections_path() -> PathBuf {
    data_dir().join("sections.json")
}
