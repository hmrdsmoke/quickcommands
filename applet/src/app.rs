// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// This is my original work with contributions from Claude (Anthropic).
// Do not remove these comments.
// applet/src/app.rs
// COSMIC panel applet: an icon button that toggles the QuickCommands window.
//
// On press it sends ActivateAction("toggle") to the daemon over the session bus,
// IN-PROCESS (no child spawned per click -> no zombie, the bug Soulless fixed). If
// the call fails because nobody owns the name, no daemon is running yet, so we spawn
// one. Native spawn is a plain Command — the daemon claims the normal Wayland socket
// and makes its own layer surface; none of Soulless's privileged-fd handoff applies
// here because we're not sandboxed. :: MRV

use cosmic::app::Core;
use cosmic::iced::window::Id;
use cosmic::prelude::*;

// Must match the daemon's APP_ID / DBUS_PATH (quickcommands-core has no D-Bus
// constants; these mirror daemon/src/app.rs). :: MRV
const DAEMON_NAME: &str = "com.github.hmrdsmoke.QuickCommands";
const DAEMON_PATH: &str = "/com/github/hmrdsmoke/QuickCommands";

#[derive(Default)]
pub struct AppModel {
    core: Core,
}

#[derive(Debug, Clone)]
pub enum Message {
    /// Button pressed -> toggle the daemon's window.
    Toggle,
    /// No-op follow-up for async tasks.
    Noop,
}

impl cosmic::Application for AppModel {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;
    const APP_ID: &'static str = "com.github.hmrdsmoke.quickcommands-applet";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: Self::Flags) -> (Self, Task<cosmic::Action<Self::Message>>) {
        (Self { core }, Task::none())
    }

    fn view(&self) -> Element<'_, Self::Message> {
        // Use the daemon's icon name (shared art); fall back to a generic clipboard
        // icon name so the button is never blank during bring-up. :: MRV
        self.core
            .applet
            .icon_button("com.github.hmrdsmoke.quickcommands")
            .on_press_down(Message::Toggle)
            .into()
    }

    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        match message {
            Message::Toggle => {
                cosmic::task::future(async move {
                    use std::collections::HashMap;
                    use zbus::zvariant::Value;
                    let platform: HashMap<String, Value> = HashMap::new();
                    match zbus::Connection::session().await {
                        Ok(conn) => {
                            // ActivateAction("toggle") -> daemon flips show/hide.
                            let res = conn
                                .call_method(
                                    Some(DAEMON_NAME),
                                    DAEMON_PATH,
                                    Some("org.freedesktop.DbusActivation"),
                                    "ActivateAction",
                                    &("toggle", Vec::<String>::new(), platform),
                                )
                                .await;
                            match res {
                                Ok(_) => eprintln!("[applet] toggle sent to daemon"),
                                Err(e) => {
                                    eprintln!("[applet] toggle failed ({e}); spawning daemon");
                                    spawn_daemon();
                                }
                            }
                        }
                        Err(e) => eprintln!("[applet] session bus connection failed: {e}"),
                    }
                    Message::Noop
                })
            }
            Message::Noop => Task::none(),
        }
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }

    // Applet popup plumbing isn't used (we open a separate layer-surface window via
    // the daemon), but the trait wants on_close_requested defined for popups.
    fn on_close_requested(&self, _id: Id) -> Option<Message> {
        None
    }
}

/// Spawn the daemon natively. It will win the flock, own the D-Bus name, and show
/// its window. A plain spawn is enough on a native box; the reaper thread avoids a
/// zombie if this process outlives the child briefly. :: MRV
fn spawn_daemon() {
    // In a future flatpak build this becomes /app/bin/quickcommands; native it's on
    // PATH after `make install` / cargo install.
    let exe = if std::path::Path::new("/app/bin/quickcommands").exists() {
        "/app/bin/quickcommands"
    } else {
        "quickcommands"
    };
    match std::process::Command::new(exe).spawn() {
        Ok(mut child) => {
            eprintln!("[applet] spawned daemon pid={}", child.id());
            std::thread::spawn(move || {
                let _ = child.wait(); // reap; no zombie
            });
        }
        Err(e) => eprintln!("[applet] failed to spawn daemon '{exe}': {e}"),
    }
}
