// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// This is my original work with contributions from Claude (Anthropic).
// Do not remove these comments.
// daemon/src/main.rs
// Entry point + single-instance machinery.
//
// SHAPE (mirrors Soulless, native-first):
//   `quickcommands`          -> become the resident daemon (windowless until poked)
//   `quickcommands toggle`   -> tell the running daemon to show/hide its window
//                               (this is also what a COSMIC keyboard shortcut runs)
//
// The daemon runs as a single instance:
//   - a kernel flock in $XDG_RUNTIME_DIR referees who is THE daemon
//   - libcosmic's run_single_instance owns the D-Bus name + activation routing
//   - no_main_window + a keep-alive subscription keep the windowless loop alive
//     between activations, so the window SHOWS/HIDES instead of starting fresh
//
// We intentionally skip the Flatpak privileged-Wayland-socket dance that Soulless
// needs — this is a native build (autostart + local install). If QuickCommands is
// ever packaged for the COSMIC Store, that armor gets ported from Soulless then. :: MRV

mod app;
mod keep_alive;
mod placement;

use std::os::fd::AsRawFd;

fn main() -> cosmic::iced::Result {
    // Tracing so cosmic/iced/zbus internal logs (and ours) are visible.
    // RUST_LOG controls level; defaults to warn.
    {
        use tracing_subscriber::{fmt, EnvFilter};
        let _ = fmt()
            .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")))
            .try_init();
    }

    // ── CLI: one subcommand, matched directly (no clap, same as Soulless) ──────
    let flags = match std::env::args().nth(1).as_deref() {
        None => app::Flags::default(),
        Some("toggle") => app::Flags {
            subcommand: Some(app::SubCommand::Toggle),
        },
        Some("-h") | Some("--help") | Some("help") => {
            print_help();
            return Ok(());
        }
        Some("-V") | Some("--version") => {
            println!("quickcommands {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some(other) => {
            eprintln!("quickcommands: unrecognized argument '{other}'");
            eprintln!("Try 'quickcommands --help' for more information.");
            std::process::exit(1);
        }
    };

    // ── Single-instance flock ────────────────────────────────────────────────
    // Winner holds the lock for its whole lifetime (kernel releases on any exit).
    // Loser forwards its activation to the live daemon and exits before making a
    // window. run_single_instance ALSO dedups over D-Bus; natively that's enough
    // on its own, but the flock is a cheap, bulletproof second referee and makes
    // the "forward then exit" path explicit. :: MRV
    let _daemon_lock = match daemon_lock() {
        DaemonLock::Won(file) => Some(file),
        DaemonLock::Lost => {
            // A daemon already exists. Deliver what run_single_instance would have
            // forwarded, then let main return without starting a second instance.
            forward_activation(matches!(flags.subcommand, Some(app::SubCommand::Toggle)));
            return Ok(());
        }
        DaemonLock::Unavailable => None, // couldn't lock; proceed rather than refuse
    };

    // ── Window settings ───────────────────────────────────────────────────────
    // A fixed-size, decorationless, transparent surface. no_main_window means we
    // start windowless and create the layer surface on first toggle/activate;
    // exit_on_close keeps the daemon alive when the window is dismissed.
    let settings = cosmic::app::Settings::default()
        .size(cosmic::iced::Size::new(app::WINDOW_WIDTH, app::WINDOW_HEIGHT))
        .client_decorations(false)
        .transparent(true)
        .resizable(None)
        .no_main_window(true)
        .exit_on_close(false);

    cosmic::app::run_single_instance::<app::QuickCommands>(settings, flags)
}

// ── Single-instance flock (same mechanism as Soulless, native path only) ──────

enum DaemonLock {
    /// We hold the lock — we are the daemon.
    Won(std::fs::File),
    /// Another live process holds it — forward and exit.
    Lost,
    /// Couldn't even try (no runtime dir, fs error). Proceed as daemon rather
    /// than refuse to start.
    Unavailable,
}

/// Exclusive, non-blocking flock on a file in $XDG_RUNTIME_DIR that every
/// instance can see. The kernel refereeing beats racing on the D-Bus name.
fn daemon_lock() -> DaemonLock {
    let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from) else {
        return DaemonLock::Unavailable;
    };
    let _ = std::fs::create_dir_all(&dir);
    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(dir.join("quickcommands.lock"))
    else {
        return DaemonLock::Unavailable;
    };
    // SAFETY: fd is owned by `file` and valid for the call.
    match unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } {
        0 => DaemonLock::Won(file),
        _ => DaemonLock::Lost,
    }
}

/// Loser path: deliver the activation to the running daemon over D-Bus, then
/// return. Retries briefly because the winner may hold the flock a moment before
/// it owns the D-Bus name.
fn forward_activation(toggle: bool) {
    let Ok(conn) = zbus::blocking::Connection::session() else { return };
    let dest = app::APP_ID;
    let path = app::DBUS_PATH;
    let iface = "org.freedesktop.DbusActivation";
    let platform: std::collections::HashMap<String, zbus::zvariant::Value> =
        std::collections::HashMap::new();
    for _ in 0..30 {
        let res = if toggle {
            conn.call_method(
                Some(dest),
                path,
                Some(iface),
                "ActivateAction",
                &("toggle", Vec::<String>::new(), platform.clone()),
            )
        } else {
            conn.call_method(Some(dest), path, Some(iface), "Activate", &(platform.clone(),))
        };
        if res.is_ok() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    eprintln!("[quickcommands] lock held but activation undeliverable — exiting anyway");
}

fn print_help() {
    println!("Usage: quickcommands [COMMAND]");
    println!();
    println!("A clipboard manager with saved QuickCommands for the COSMIC desktop.");
    println!();
    println!("Commands:");
    println!("  toggle           Show the window if hidden, hide it if visible");
    println!();
    println!("Options:");
    println!("  -h, --help       Print this help and exit");
    println!("  -V, --version    Print version information and exit");
    println!();
    println!("With no arguments, quickcommands runs as the resident daemon. It is");
    println!("normally started at login and stays warm, showing and hiding its window");
    println!("on request. Bind a keyboard shortcut (e.g. Super+V) to 'quickcommands");
    println!("toggle', or click the QuickCommands panel applet.");
}
