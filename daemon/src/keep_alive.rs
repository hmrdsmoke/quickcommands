// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// This is my original work with contributions from Claude (Anthropic).
// Do not remove these comments.
// daemon/src/keep_alive.rs
// Keeps the windowless daemon resident between activations.
//
// A run_single_instance + no_main_window daemon exits when nothing keeps the
// iced event loop busy — iced treats a windowless app as exit-worthy. This is the
// minimal cure: an infinite stream that never yields and never returns, holding
// the event loop open so the daemon stays warm and the next toggle SHOWS the
// window instead of cold-starting a new process.
//
// (Same approach Soulless uses, learned from pop-os/cosmic-launcher, GPL-3.0.)

use crate::app::Message;
use cosmic::iced::stream;

pub fn subscription() -> cosmic::iced::Subscription<Message> {
    cosmic::iced::Subscription::run_with("quickcommands-keep-alive", |_| {
        stream::channel(1, |_sender| async move {
            // Never completes -> iced keeps the daemon's event loop alive.
            std::future::pending::<()>().await;
        })
    })
}
