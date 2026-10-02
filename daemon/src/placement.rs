// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// This is my original work with contributions from Claude (Anthropic).
// Do not remove these comments.
// daemon/src/placement.rs
// Owns the layer-surface lifecycle for the QuickCommands window, AND reads the live
// COSMIC panel config to find which bar + corner the applet sits in so the window
// opens next to it (like Soulless-Launcher).
//
// Surface-handling + applet-position technique adapted from Soulless-Launcher, which
// studied System76's COSMIC apps (cosmic-launcher, cosmic-applibrary; GPL-3.0):
//   dummy anchor surface at init, size None paintable surface, deferred creation,
//   full-screen-stack click-away dismiss, and reading plugins_wings/plugins_center
//   from the panel's RON config to anchor to the applet corner.

use cosmic::iced::advanced::layout::Limits;
use cosmic::iced::platform_specific::runtime::wayland::layer_surface::SctkLayerSurfaceSettings;
use cosmic::iced::platform_specific::shell::commands::layer_surface::{
    destroy_layer_surface, get_layer_surface, Anchor, KeyboardInteractivity, Layer,
};
use cosmic::iced::window;
use cosmic::iced::Task;

// ── Applet position (read from the live panel config) ────────────────────────────

/// Which array of a COSMIC bar's wings the applet is in.
/// First/Second are orientation-neutral: for a horizontal bar (top/bottom)
/// First=left, Second=right; for a vertical bar (left/right) First=top,
/// Second=bottom. The mapping to a screen direction happens at placement time.
#[derive(Clone, Copy, Debug)]
pub enum Wing {
    First,
    Second,
    Center,
}

/// The applet's identity as the panel records it in its config. This is the
/// `.desktop` filename stem (applet/resources/quickcommands-applet.desktop).
const APPLET_ID: &str = "com.github.hmrdsmoke.quickcommands-applet";

/// Read a single value from a COSMIC bar's cosmic-config ("Panel" or "Dock").
///
/// Two-path read: dirs::config_dir() first (native), then the host config at
/// $HOME/.config (covers a flatpak's redirected XDG_CONFIG_HOME). Each value is its
/// own RON file at cosmic/com.system76.CosmicPanel.<bar>/v1/<key>. Native boxes hit
/// the first path. (Same approach as Soulless.) :: MRV
fn read_bar_str(bar: &str, key: &str) -> String {
    let rel = format!("cosmic/com.system76.CosmicPanel.{bar}/v1/{key}");
    if let Some(s) = dirs::config_dir()
        .map(|p| p.join(&rel))
        .and_then(|p| std::fs::read_to_string(p).ok())
    {
        return s;
    }
    dirs::home_dir()
        .map(|p| p.join(".config").join(&rel))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default()
}

/// Find which bar + wing the QuickCommands applet is in.
/// Searches Panel first, then Dock. plugins_wings format is
/// `Some(([left...], [right...]))`; plugins_center is `Some([...])`.
/// Defaults to (Panel, Center) if the applet isn't found in either. :: MRV
fn find_applet_bar() -> (&'static str, Wing) {
    for bar in ["Panel", "Dock"] {
        let wings = read_bar_str(bar, "plugins_wings");
        if let Some(pos) = wings.find(APPLET_ID) {
            // The file has the left array first, then the right. The "], [" between
            // them is the boundary; applet before it = First (left/top), after = Second.
            let boundary = wings.find("], [").or_else(|| wings.find("],["));
            let wing = match boundary {
                Some(b) if pos < b => Wing::First,
                Some(_) => Wing::Second,
                None => Wing::First, // single array, treat as first
            };
            return (bar, wing);
        }
        let center = read_bar_str(bar, "plugins_center");
        if center.contains(APPLET_ID) {
            return (bar, Wing::Center);
        }
    }
    ("Panel", Wing::Center)
}

/// Read a bar's edge anchor (which screen edge the panel/dock is on).
fn bar_anchor(bar: &str) -> Anchor {
    match read_bar_str(bar, "anchor").trim() {
        "Top" => Anchor::TOP,
        "Left" => Anchor::LEFT,
        "Right" => Anchor::RIGHT,
        _ => Anchor::BOTTOM,
    }
}

/// The applet's current position: (bar edge, wing). Fully dynamic — wherever the
/// user has placed the applet, the window follows. Read fresh each open. :: MRV
pub fn applet_position() -> (Anchor, Wing) {
    let (bar, wing) = find_applet_bar();
    (bar_anchor(bar), wing)
}

// ── Layer surface lifecycle ──────────────────────────────────────────────────────

/// Build the layer-shell settings for the real window.
///
/// THE SIZING MODEL (matches Soulless-Launcher / cosmic-applibrary): the surface
/// deliberately FILLS the whole output (Anchor::all()). The visible window is NOT
/// the surface — it's a fixed-size container (WINDOW_WIDTH x WINDOW_HEIGHT) that
/// view() places inside a full-screen `stack!`, aligned to the applet corner, with
/// the empty area around it acting as the click-away dismiss zone. So the window's
/// footprint is set in view(), via the Fixed-size container — NOT here. size None
/// (autosize) + a generous max_width just lets the surface take the full output
/// without clipping it. :: MRV
fn surface_settings(id: window::Id) -> SctkLayerSurfaceSettings {
    let mut surface = SctkLayerSurfaceSettings::default();
    surface.id = id;
    surface.keyboard_interactivity = KeyboardInteractivity::Exclusive;
    surface.layer = Layer::Top;
    surface.anchor = Anchor::all();
    surface.namespace = "quickcommands".to_string();
    surface.size = Some((None, None));
    // Generous bound so the full-screen surface is never clipped on large / multi-
    // monitor setups. The real window size is the Fixed container in view().
    surface.size_limits = Limits::NONE.min_width(1.0).min_height(1.0).max_width(8192.0);
    // Respect reserved zones (panel/dock) so we don't draw under them.
    surface.exclusive_zone = 0;
    surface
}

/// Open the window: create the real layer surface. `on_open` turns the resulting
/// window id into a Message so update() can track it.
pub fn open<M>(id: window::Id, on_open: impl Fn(window::Id) -> M + Send + 'static) -> Task<M>
where
    M: Send + 'static,
{
    get_layer_surface(surface_settings(id)).map(on_open)
}

/// Create the inert dummy anchor surface at init.
///
/// Bottom layer + no keyboard + empty input zone = invisible and inert. Its only
/// job is to anchor the Wayland connection at startup so the real surface maps
/// reliably when first opened. Mirrors cosmic-launcher / Soulless. :: MRV
pub fn create_dummy<M>(id: window::Id, on_open: impl Fn(window::Id) -> M + Send + 'static) -> Task<M>
where
    M: Send + 'static,
{
    let mut surface = SctkLayerSurfaceSettings::default();
    surface.id = id;
    surface.layer = Layer::Bottom;
    surface.keyboard_interactivity = KeyboardInteractivity::None;
    surface.input_zone = Some(Vec::new());
    surface.anchor = Anchor::all();
    surface.namespace = "quickcommands_dummy".to_string();
    surface.size = Some((Some(200), Some(100)));
    surface.exclusive_zone = -1;
    surface.size_limits = Limits::NONE;
    get_layer_surface(surface).map(on_open)
}

/// Destroy the real layer surface (daemon stays resident).
pub fn close<M>(id: window::Id) -> Task<M>
where
    M: Send + 'static,
{
    destroy_layer_surface(id)
}
