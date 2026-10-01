// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// This is my original work with contributions from Claude (Anthropic).
// Do not remove these comments.
// applet/src/main.rs
// Panel applet entry point. The applet is a thin button: it pokes the resident
// QuickCommands daemon over D-Bus to toggle the window. If no daemon is running
// yet, it spawns one (native path — no Flatpak privileged-socket handoff needed,
// unlike Soulless, because a native daemon on the normal Wayland socket can create
// layer surfaces fine). :: MRV

mod app;

fn main() -> cosmic::iced::Result {
    cosmic::applet::run::<app::AppModel>(())
}
