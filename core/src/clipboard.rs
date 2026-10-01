// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// This is my original work with contributions from Claude (Anthropic).
// Do not remove these comments.
// core/src/clipboard.rs
// The clipboard engine: read the current selection, write a new selection, and
// a simple poll-based watcher that notices when the clipboard text changes.
//
// WHY POLLING (and not the event API):
// wl-clipboard-rs ships a `watch` module, but its exact API moves between 0.x
// releases. A dumb 500ms poll of get_contents is what a lot of clipboard
// managers actually do, it needs no privileged protocol, and it can't break on
// a crate update. If this ever shows up as laggy or wasteful we can swap in the
// event-driven Watcher behind the same `watch()` signature. Working first. :: MRV

use std::time::Duration;

/// How often the watcher checks the clipboard. 500ms is imperceptible for a
/// "grab what I just copied" tool and costs almost nothing. :: MRV
pub const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Read the current clipboard text, if any.
///
/// Returns:
///   - Ok(Some(text)) when the regular clipboard holds text
///   - Ok(None)       when it's empty or holds no text mime type (e.g. an image)
///   - Err(_)         only on a real Wayland/protocol failure
///
/// We ask specifically for a text mime type; a non-text selection (image, files)
/// comes back as "no text" (Ok(None)) rather than an error, because for history
/// purposes an image copy is simply "nothing to remember as text" for now.
pub fn current_text() -> Result<Option<String>, String> {
    use wl_clipboard_rs::paste::{get_contents, ClipboardType, Error, MimeType, Seat};

    match get_contents(ClipboardType::Regular, Seat::Unspecified, MimeType::Text) {
        Ok((mut reader, _mime)) => {
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut reader, &mut buf)
                .map_err(|e| format!("reading clipboard pipe: {e}"))?;
            let text = String::from_utf8_lossy(&buf).into_owned();
            Ok(Some(text))
        }
        // Empty clipboard or no text offered -> nothing to record, not an error.
        Err(Error::NoSeats) | Err(Error::ClipboardEmpty) | Err(Error::NoMimeType) => Ok(None),
        Err(e) => Err(format!("clipboard read failed: {e}")),
    }
}

/// Put `text` on the regular clipboard. This is what click-to-copy calls.
///
/// NOTE on how wl-clipboard works: setting the clipboard forks a tiny server
/// that keeps offering the data for as long as something might paste it. The
/// `Options`/`copy` call here handles that; the data stays available after this
/// returns. We copy as an auto-detected mime type so plain text pastes cleanly
/// everywhere.
pub fn set_text(text: &str) -> Result<(), String> {
    use wl_clipboard_rs::copy::{MimeType, Options, Source};

    let opts = Options::new();
    opts.copy(
        Source::Bytes(text.to_owned().into_bytes().into()),
        MimeType::Autodetect,
    )
    .map_err(|e| format!("clipboard write failed: {e}"))
}

/// Poll-based clipboard watcher.
///
/// Blocks the calling thread forever, calling `on_change(new_text)` each time the
/// clipboard text changes to something non-empty and different from last time.
/// Run this on a dedicated std::thread from the daemon; it never returns.
///
/// The daemon owns history, so the callback just hands over the new text and the
/// daemon decides what to do with it (record + save + refresh the window).
pub fn watch<F>(mut on_change: F) -> !
where
    F: FnMut(String),
{
    let mut last: Option<String> = None;

    loop {
        match current_text() {
            Ok(Some(text)) => {
                let changed = last.as_ref() != Some(&text);
                if changed && !text.trim().is_empty() {
                    last = Some(text.clone());
                    on_change(text);
                }
            }
            Ok(None) => {
                // Clipboard empty / non-text. Reset `last` so that if the user
                // copies the SAME text again after an image copy, we still catch
                // it as a change.
                last = None;
            }
            Err(e) => {
                // Transient failures happen (compositor restart, race). Log once
                // and keep polling rather than dying.
                tracing::warn!("[clipboard] watch poll error: {e}");
            }
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}
