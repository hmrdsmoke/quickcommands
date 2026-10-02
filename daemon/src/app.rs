// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// This is my original work with contributions from Claude (Anthropic).
// Do not remove these comments.
// daemon/src/app.rs
// The resident daemon: application model, layer-surface show/hide, clipboard
// wiring, and the window view (history + QuickCommands sections + click-to-copy).
//
// Patterns for the layer-surface lifecycle, the event-filtering subscription, the
// dummy anchor surface, and the dbus_activation toggle were adapted from
// Soulless-Launcher (which studied System76's cosmic-launcher / cosmic-applibrary,
// GPL-3.0). This is the stripped-down version: no search index, no monitors, no
// compositor popups — just a pop-up-and-dismiss clipboard window. :: MRV

use cosmic::iced::{event, window::Id, Subscription};
use cosmic::prelude::*;
use cosmic::widget;
use std::sync::mpsc;
use std::sync::Mutex;

use quickcommands_core::storage::{Command, History, Sections};
use quickcommands_core::{clipboard, storage};

// ── Window dimensions ──────────────────────────────────────────────────────────
pub const WINDOW_WIDTH: f32 = 900.0;
pub const WINDOW_HEIGHT: f32 = 700.0;

// ── D-Bus identity ──────────────────────────────────────────────────────────────
// Hyphen-free APP_ID is REQUIRED for run_single_instance: cosmic derives a D-Bus
// object path (APP_ID.replace('.', "/")) and well-known name from it, and D-Bus
// forbids hyphens. The external identity (.desktop, binary) can keep "quickcommands".
// (Same rule Soulless documents.) :: MRV
pub const APP_ID: &str = "com.github.hmrdsmoke.QuickCommands";
pub const DBUS_PATH: &str = "/com/github/hmrdsmoke/QuickCommands";

// ── Clipboard -> UI bridge ──────────────────────────────────────────────────────
// The clipboard watcher runs on its own std::thread (clipboard::watch blocks
// forever). It can't call into the iced loop directly, so it sends new copies down
// this channel. A subscription drains the receiver and turns each into a Message.
//
// The receiver lives in a global Mutex<Option<..>> because the subscription closure
// (which owns the receiver) is created by iced, not by us, and we need to hand the
// receiver to it exactly once. The watcher thread + sender are spawned in init().
static CLIP_RX: Mutex<Option<mpsc::Receiver<String>>> = Mutex::new(None);

// ── Messages ────────────────────────────────────────────────────────────────────
#[derive(Debug, Clone)]
pub enum Message {
    /// A new clipboard copy arrived from the watcher thread.
    ClipboardChanged(String),
    /// The real layer surface finished being requested (carries its id).
    WindowOpened(Id),
    /// Deferred show: create the surface on the next loop cycle (see dbus_activation).
    ShowSurface,
    /// A raw iced/wayland event we care about (dismiss-on-unfocus, Esc).
    WindowEvent(cosmic::iced::Event),
    /// User clicked a history entry -> copy its text back to the clipboard.
    CopyHistory(uuid::Uuid),
    /// User clicked a saved command -> copy its text to the clipboard.
    CopyCommand(uuid::Uuid),
    /// Typed in the search/filter box.
    SearchChanged(String),
    /// Collapse/expand a section (by id).
    ToggleSection(uuid::Uuid),
    /// Dismiss the window (click-away on the background layer, or Esc).
    Dismiss,
    /// No-op for tasks with no follow-up (absorbs clicks on the window zone).
    Noop,
}

// ── Single-instance flags (minimal, no clap) ─────────────────────────────────────
// run_single_instance requires Flags: CosmicFlags. The applet activates over D-Bus,
// so CLI parsing is just the one `toggle` subcommand. (Shape from cosmic-launcher
// via Soulless.) :: MRV
#[derive(Debug, Clone)]
pub enum SubCommand {
    Toggle,
}

impl std::fmt::Display for SubCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Lowercase: goes on the wire as the D-Bus action name; matched in
        // dbus_activation as "toggle".
        match self {
            SubCommand::Toggle => write!(f, "toggle"),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Flags {
    pub subcommand: Option<SubCommand>,
}

impl cosmic::app::CosmicFlags for Flags {
    type SubCommand = SubCommand;
    type Args = Vec<String>;

    fn action(&self) -> Option<&Self::SubCommand> {
        self.subcommand.as_ref()
    }
}

// ── Application model ────────────────────────────────────────────────────────────
pub struct QuickCommands {
    core: cosmic::Core,
    /// Rolling clipboard history (newest first).
    history: History,
    /// User's saved command sections.
    sections: Sections,
    /// Current filter text; filters both history and commands.
    search: String,
    /// Stable id for the real window surface, reused for create + destroy.
    window_id: Id,
    /// Inert anchor surface id (created at init).
    dummy_id: Option<Id>,
    /// Whether the real surface is currently open. Guards close() so a second
    /// dismiss (e.g. Unfocused right after Esc) is a safe no-op.
    surface_open: bool,
}

impl QuickCommands {
    /// Guarded dismiss: only destroy the surface if it's actually open.
    fn dismiss(&mut self) -> Task<cosmic::Action<Message>> {
        if self.surface_open {
            self.surface_open = false;
            crate::placement::close(self.window_id)
        } else {
            Task::none()
        }
    }

    /// Show the window fresh: reset the filter and create the surface.
    fn show(&mut self) -> Task<cosmic::Action<Message>> {
        // Reload from disk so a new window reflects any edits made elsewhere
        // (and picks up history the watcher saved while we were hidden).
        self.history = History::load();
        self.sections = Sections::load();
        self.search.clear();
        self.surface_open = true;
        crate::placement::open(self.window_id, Message::WindowOpened).map(cosmic::Action::App)
    }
}

impl cosmic::Application for QuickCommands {
    type Executor = cosmic::executor::Default;
    type Flags = Flags;
    type Message = Message;
    const APP_ID: &'static str = APP_ID;

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    fn init(core: cosmic::Core, _flags: Self::Flags) -> (Self, Task<cosmic::Action<Self::Message>>) {
        // Load persisted state up front.
        let history = History::load();
        let sections = Sections::load();

        // Spawn the clipboard watcher thread and stash its receiver for the
        // subscription to pick up. Done once, here. The thread blocks forever in
        // clipboard::watch, sending each new copy down the channel.
        let (tx, rx) = mpsc::channel::<String>();
        if let Ok(mut slot) = CLIP_RX.lock() {
            *slot = Some(rx);
        }
        std::thread::Builder::new()
            .name("quickcommands-clipboard".into())
            .spawn(move || {
                clipboard::watch(|text| {
                    // If the UI thread is gone the send fails; the watcher simply
                    // keeps running harmlessly.
                    let _ = tx.send(text);
                });
            })
            .expect("spawn clipboard watcher thread");

        let window_id = Id::unique();
        let dummy_id = Id::unique();

        (
            Self {
                core,
                history,
                sections,
                search: String::new(),
                window_id,
                dummy_id: Some(dummy_id),
                surface_open: false,
            },
            // Anchor the Wayland connection with the inert dummy surface so the
            // real surface maps reliably on first open. (cosmic-launcher pattern.)
            crate::placement::create_dummy(dummy_id, |_id| cosmic::Action::App(Message::Noop)),
        )
    }

    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        match message {
            Message::ClipboardChanged(text) => {
                // Record the new copy. If it actually changed history, persist it.
                // We do NOT open the window — capturing is silent/background.
                if self.history.record(text) {
                    if let Err(e) = self.history.save() {
                        tracing::warn!("[quickcommands] failed to save history: {e}");
                    }
                }
                Task::none()
            }

            Message::ShowSurface => {
                // Deferred creation path (from dbus Activate): create now.
                crate::placement::open(self.window_id, Message::WindowOpened)
                    .map(cosmic::Action::App)
            }

            Message::WindowOpened(_id) => {
                // Surface requested. Mapping completes via the compositor's
                // Opened/Resized events (filtered in subscription); nothing to do.
                Task::none()
            }

            Message::CopyHistory(id) => {
                if let Some(entry) = self.history.entries.iter().find(|e| e.id == id) {
                    if let Err(e) = clipboard::set_text(&entry.text) {
                        tracing::warn!("[quickcommands] copy failed: {e}");
                    }
                }
                // Copying is the whole point -> dismiss after.
                self.dismiss()
            }

            Message::CopyCommand(id) => {
                let found = self
                    .sections
                    .sections
                    .iter()
                    .flat_map(|s| &s.commands)
                    .find(|c| c.id == id)
                    .map(|c| c.command.clone());
                if let Some(cmd) = found {
                    if let Err(e) = clipboard::set_text(&cmd) {
                        tracing::warn!("[quickcommands] copy failed: {e}");
                    }
                }
                self.dismiss()
            }

            Message::SearchChanged(s) => {
                self.search = s;
                Task::none()
            }

            Message::ToggleSection(id) => {
                if let Some(sec) = self.sections.sections.iter_mut().find(|s| s.id == id) {
                    sec.collapsed = !sec.collapsed;
                    // Persist the collapsed state so it sticks between launches.
                    if let Err(e) = self.sections.save() {
                        tracing::warn!("[quickcommands] failed to save sections: {e}");
                    }
                }
                Task::none()
            }

            Message::WindowEvent(ev) => {
                use cosmic::iced::event::wayland::Event as WaylandEvent;
                use cosmic::iced::event::PlatformSpecific;
                use cosmic::iced::keyboard::{Event as KeyEvent, Key};
                match ev {
                    // Esc dismisses.
                    cosmic::iced::Event::Keyboard(KeyEvent::KeyPressed { key, .. }) => {
                        if key == Key::Named(cosmic::iced::keyboard::key::Named::Escape) {
                            return self.dismiss();
                        }
                        Task::none()
                    }
                    // Click-away: the layer surface loses focus -> dismiss. Same
                    // model Soulless uses (full-screen-stack unfocus dismiss).
                    cosmic::iced::Event::PlatformSpecific(PlatformSpecific::Wayland(
                        WaylandEvent::Layer(layer_event, ..),
                    )) => {
                        if matches!(layer_event, cosmic::iced::event::wayland::LayerEvent::Unfocused)
                        {
                            return self.dismiss();
                        }
                        Task::none()
                    }
                    _ => Task::none(),
                }
            }

            Message::Dismiss => self.dismiss(),
            Message::Noop => Task::none(),
        }
    }

    fn dbus_activation(
        &mut self,
        msg: cosmic::dbus_activation::Message,
    ) -> Task<cosmic::Action<Self::Message>> {
        use cosmic::dbus_activation::Details;
        match msg.msg {
            // Activate (plain applet click / `quickcommands`): show if hidden,
            // no-op if already open. Defer creation one cycle so it maps.
            Details::Activate => {
                if self.surface_open {
                    Task::none()
                } else {
                    self.history = History::load();
                    self.sections = Sections::load();
                    self.search.clear();
                    self.surface_open = true;
                    cosmic::task::message(cosmic::Action::App(Message::ShowSurface))
                }
            }
            // ActivateAction("toggle") (hotkey / `quickcommands toggle`): flip.
            Details::ActivateAction { action, .. } if action == "toggle" => {
                if self.surface_open {
                    self.dismiss()
                } else {
                    self.show()
                }
            }
            _ => Task::none(),
        }
    }

    fn view(&self) -> Element<'_, Self::Message> {
        // view() is required but the real content is per-surface in view_window.
        // The daemon is windowless at rest, so this is never the layer surface.
        widget::text("").into()
    }

    fn view_window(&self, id: Id) -> Element<'_, Self::Message> {
        // The dummy anchor surface renders nothing (stays invisible).
        if Some(id) == self.dummy_id {
            return widget::column![].into();
        }
        self.window_view()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        Subscription::batch([
            // Keep the windowless daemon resident between activations.
            crate::keep_alive::subscription(),
            // Drain the clipboard watcher channel into ClipboardChanged messages.
            clipboard_subscription(),
            // Filter raw events: only the few we handle return Some, so we don't
            // re-arm a render flood. (Critical — same lesson as Soulless.)
            event::listen_with(|ev, _status, _id| match &ev {
                cosmic::iced::Event::PlatformSpecific(
                    cosmic::iced::event::PlatformSpecific::Wayland(
                        cosmic::iced::event::wayland::Event::Layer(..),
                    ),
                ) => Some(Message::WindowEvent(ev)),
                cosmic::iced::Event::Keyboard(cosmic::iced::keyboard::Event::KeyPressed {
                    ..
                }) => Some(Message::WindowEvent(ev)),
                // Everything else (frame/redraw) -> None; breaks the flood.
                _ => None,
            }),
        ])
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        // Transparent app background so the layer surface shows our own card, not
        // an opaque rectangle over the desktop.
        None
    }
}

// ── The window content ───────────────────────────────────────────────────────────
impl QuickCommands {
    fn window_view(&self) -> Element<'_, Message> {
        let spacing = cosmic::theme::active().cosmic().spacing;

        // Search / filter box at the top.
        let search = widget::search_input("Filter history and commands", &self.search)
            .on_input(Message::SearchChanged)
            .on_clear(Message::SearchChanged(String::new()));

        let needle = self.search.to_lowercase();

        // ── Column 1: Recent copies ────────────────────────────────────────────
        let mut recent = widget::column![].spacing(spacing.space_xxs);
        recent = recent.push(section_header("Recent"));
        let mut shown = 0usize;
        for entry in &self.history.entries {
            if !needle.is_empty() && !entry.text.to_lowercase().contains(&needle) {
                continue;
            }
            shown += 1;
            recent = recent.push(history_row(entry));
        }
        if shown == 0 {
            recent = recent.push(empty_hint(if self.history.entries.is_empty() {
                "Nothing copied yet. Copy something and it shows up here."
            } else {
                "No copies match your filter."
            }));
        }

        // ── Column 2: Git ──────────────────────────────────────────────────────
        // The single section literally named "Git" (case-insensitive). If there
        // isn't one, the column shows a gentle hint.
        let mut git = widget::column![].spacing(spacing.space_xs);
        git = git.push(section_header("Git"));
        let git_section = self
            .sections
            .sections
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case("git"));
        match git_section {
            Some(sec) => {
                if let Some(block) = self.section_commands(sec, &needle, spacing) {
                    git = git.push(block);
                } else {
                    git = git.push(empty_hint("No Git commands match your filter."));
                }
            }
            None => {
                git = git.push(empty_hint(
                    "No \"Git\" section yet. Add one in sections.json.",
                ));
            }
        }

        // ── Column 3: Projects ─────────────────────────────────────────────────
        // Every section EXCEPT the Git one, each as its own labeled group stacked
        // down the column.
        let mut projects = widget::column![].spacing(spacing.space_s);
        projects = projects.push(section_header("Projects"));
        let mut any_project = false;
        for sec in self
            .sections
            .sections
            .iter()
            .filter(|s| !s.name.eq_ignore_ascii_case("git"))
        {
            // A collapsible per-section header (toggles like before).
            let caret = if sec.collapsed { "▸" } else { "▾" };
            let header = widget::button::custom(
                widget::row![]
                    .spacing(spacing.space_xxs)
                    .push(widget::text(caret))
                    .push(widget::text::heading(sec.name.clone())),
            )
            .on_press(Message::ToggleSection(sec.id))
            .class(cosmic::theme::Button::Text)
            .width(cosmic::iced::Length::Fill);

            // When filtering, only show this project group if something matches.
            let block = self.section_commands(sec, &needle, spacing);
            if !needle.is_empty() && block.is_none() {
                continue;
            }
            any_project = true;
            projects = projects.push(header);
            if !sec.collapsed || !needle.is_empty() {
                if let Some(b) = block {
                    projects = projects.push(b);
                }
            }
        }
        if !any_project {
            projects = projects.push(empty_hint(if needle.is_empty() {
                "No project sections yet. Add some in sections.json."
            } else {
                "No project commands match your filter."
            }));
        }

        // Each column is its own independent scrollable, splitting the width evenly.
        // (Built inline rather than via a closure — a closure can't express the
        // borrow lifetime tying the column content to the returned container.) :: MRV
        let recent_col = widget::container(
            widget::scrollable(recent)
                .height(cosmic::iced::Length::Fill)
                .width(cosmic::iced::Length::Fill),
        )
        .width(cosmic::iced::Length::FillPortion(1))
        .height(cosmic::iced::Length::Fill);

        let git_col = widget::container(
            widget::scrollable(git)
                .height(cosmic::iced::Length::Fill)
                .width(cosmic::iced::Length::Fill),
        )
        .width(cosmic::iced::Length::FillPortion(1))
        .height(cosmic::iced::Length::Fill);

        let projects_col = widget::container(
            widget::scrollable(projects)
                .height(cosmic::iced::Length::Fill)
                .width(cosmic::iced::Length::Fill),
        )
        .width(cosmic::iced::Length::FillPortion(1))
        .height(cosmic::iced::Length::Fill);

        let body = widget::row![]
            .spacing(spacing.space_m)
            .push(recent_col)
            .push(git_col)
            .push(projects_col)
            .height(cosmic::iced::Length::Fill)
            .width(cosmic::iced::Length::Fill);

        // THE WINDOW SIZING MODEL (matches Soulless-Launcher / cosmic-applibrary):
        //
        // The layer surface fills the whole screen on purpose (Anchor::all()), and
        // this view's root is a full-screen `stack!` of TWO layers:
        //   1. a transparent Fill background that dismisses on click (click-away)
        //   2. the actual window: a container with a HARD Fixed width AND height
        //      (WINDOW_WIDTH x WINDOW_HEIGHT), wrapped in a mouse_area that ABSORBS
        //      clicks so they don't fall through to the dismiss layer, and aligned
        //      top-center within the Fill area.
        //
        // The visible window is the fixed-size box; the empty space around it is the
        // click-away zone. This is why the size is a Fixed container here, NOT a
        // surface size — trying to shrink the surface itself is what caused the
        // full-screen window. Change the window footprint via WINDOW_WIDTH/HEIGHT. :: MRV
        let card = widget::container(
            widget::column![]
                .spacing(spacing.space_s)
                .push(search)
                .push(body),
        )
        .padding(spacing.space_m)
        .width(cosmic::iced::Length::Fixed(WINDOW_WIDTH))
        .height(cosmic::iced::Length::Fixed(WINDOW_HEIGHT))
        .class(cosmic::theme::Container::Card);

        // Absorb clicks on the window so they don't dismiss it.
        let window_zone = cosmic::iced::widget::mouse_area(card).on_press(Message::Noop);

        // A little breathing room from the panel edge.
        let gap = spacing.space_s as f32;

        // Place the window next to the applet (Soulless pattern). Read where the
        // applet lives from the live panel config and map bar-edge + wing to an
        // alignment within the full-screen surface:
        //   - horizontal bar (Top/Bottom): wing runs left/right, edge is top/bottom
        //   - vertical bar  (Left/Right):  wing runs top/bottom,  edge is left/right
        // Center wing -> centered on that axis. :: MRV
        use cosmic::iced::platform_specific::shell::commands::layer_surface::Anchor as LSAnchor;
        use cosmic::iced::Alignment;
        let (edge, wing) = crate::placement::applet_position();
        let horizontal_bar = matches!(edge, LSAnchor::TOP | LSAnchor::BOTTOM);
        let (align_x, align_y) = if horizontal_bar {
            let ax = match wing {
                crate::placement::Wing::First => Alignment::Start,
                crate::placement::Wing::Second => Alignment::End,
                crate::placement::Wing::Center => Alignment::Center,
            };
            let ay = if matches!(edge, LSAnchor::TOP) { Alignment::Start } else { Alignment::End };
            (ax, ay)
        } else {
            let ay = match wing {
                crate::placement::Wing::First => Alignment::Start,
                crate::placement::Wing::Second => Alignment::End,
                crate::placement::Wing::Center => Alignment::Center,
            };
            let ax = if matches!(edge, LSAnchor::LEFT) { Alignment::Start } else { Alignment::End };
            (ax, ay)
        };

        cosmic::iced::widget::stack![
            // Background dismiss layer: any click outside the window closes it.
            cosmic::iced::widget::mouse_area(
                widget::container(cosmic::iced::widget::space::horizontal())
                    .width(cosmic::iced::Length::Fill)
                    .height(cosmic::iced::Length::Fill)
            )
            .on_press(Message::Dismiss),
            // The window itself, aligned to the applet corner within the surface.
            widget::container(window_zone)
                .align_x(align_x)
                .align_y(align_y)
                .padding(cosmic::iced::Padding::from(gap))
                .width(cosmic::iced::Length::Fill)
                .height(cosmic::iced::Length::Fill)
        ]
        .width(cosmic::iced::Length::Fill)
        .height(cosmic::iced::Length::Fill)
        .into()
    }

    /// Render a section's commands as a click-to-copy list, filtered by `needle`.
    /// Returns None when the filter is active and nothing in the section matches
    /// (so the caller can skip the section/column entirely). Shared by the Git
    /// column and each Projects group. :: MRV
    fn section_commands<'a>(
        &self,
        section: &'a storage::Section,
        needle: &str,
        spacing: cosmic::cosmic_theme::Spacing,
    ) -> Option<Element<'a, Message>> {
        let matches: Vec<&Command> = section
            .commands
            .iter()
            .filter(|c| {
                needle.is_empty()
                    || c.label.to_lowercase().contains(needle)
                    || c.command.to_lowercase().contains(needle)
            })
            .collect();
        if matches.is_empty() {
            return None;
        }
        let mut inner = widget::column![].spacing(spacing.space_xxxs);
        for cmd in matches {
            inner = inner.push(command_row(cmd));
        }
        Some(
            widget::container(inner)
                .padding(cosmic::iced::Padding::from([
                    0.0,
                    0.0,
                    0.0,
                    spacing.space_s as f32,
                ]))
                .into(),
        )
    }
}

// ── Small view helpers ───────────────────────────────────────────────────────────

/// An uppercase section label.
fn section_header(label: &str) -> Element<'_, Message> {
    widget::text::heading(label).into()
}

/// A greyed hint line for empty states.
fn empty_hint(text: &str) -> Element<'_, Message> {
    widget::text::caption(text).into()
}

/// One clipboard history entry as a full-width click-to-copy button.
fn history_row(entry: &storage::HistoryEntry) -> Element<'_, Message> {
    // Collapse whitespace to a single line for the preview; the full text is what
    // gets copied. Trim to a sane length so one giant paste doesn't blow out the row.
    let preview = preview_text(&entry.text, 80);
    widget::button::custom(widget::text(preview).width(cosmic::iced::Length::Fill))
        .on_press(Message::CopyHistory(entry.id))
        .class(cosmic::theme::Button::MenuItem)
        .width(cosmic::iced::Length::Fill)
        .into()
}

/// One saved command as a full-width click-to-copy row: label + the command in
/// monospace so it reads like what it is.
fn command_row(cmd: &storage::Command) -> Element<'_, Message> {
    let row = widget::row![]
        .spacing(8.0)
        .push(widget::text(cmd.label.clone()).width(cosmic::iced::Length::FillPortion(2)))
        .push(
            widget::text::monotext(preview_text(&cmd.command, 60))
                .width(cosmic::iced::Length::FillPortion(3)),
        );
    widget::button::custom(row)
        .on_press(Message::CopyCommand(cmd.id))
        .class(cosmic::theme::Button::MenuItem)
        .width(cosmic::iced::Length::Fill)
        .into()
}

/// One-line, length-capped preview of possibly-multiline text.
fn preview_text(text: &str, max: usize) -> String {
    let one_line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > max {
        let mut s: String = one_line.chars().take(max).collect();
        s.push('…');
        s
    } else {
        one_line
    }
}

// ── Clipboard channel subscription ───────────────────────────────────────────────

/// Bridge the clipboard watcher thread's channel into the iced message loop.
///
/// The receiver was stashed in CLIP_RX by init(). This subscription takes it (once)
/// and parks a blocking recv on a dedicated task, forwarding each copy as a
/// ClipboardChanged message. If the receiver was already taken (shouldn't happen),
/// it parks forever doing nothing.
fn clipboard_subscription() -> Subscription<Message> {
    use cosmic::iced::stream;
    use futures_util::SinkExt;

    Subscription::run_with("quickcommands-clipboard-bridge", |_| {
        stream::channel(16, |mut output: cosmic::iced::futures::channel::mpsc::Sender<Message>| async move {
            // Take the std receiver out of the global slot.
            let mut rx_opt = CLIP_RX.lock().ok().and_then(|mut s| s.take());
            if let Some(mut rx) = rx_opt.take() {
                // recv() is blocking; run it on the blocking pool and forward each
                // item. spawn_blocking needs 'static, so we MOVE the receiver in and
                // hand it back out via the returned tuple each iteration (no borrow
                // held across the await).
                loop {
                    let joined = tokio::task::spawn_blocking(move || {
                        let msg = rx.recv();
                        (rx, msg)
                    })
                    .await;

                    match joined {
                        Ok((returned_rx, Ok(text))) => {
                            rx = returned_rx;
                            if output.send(Message::ClipboardChanged(text)).await.is_err() {
                                break; // UI gone
                            }
                        }
                        // Sender dropped (RecvError) or join error -> stop forwarding.
                        _ => break,
                    }
                }
            }
            // Nothing (or done): park forever so the subscription stays alive and
            // iced doesn't treat it as completed.
            std::future::pending::<()>().await;
        })
    })
}
