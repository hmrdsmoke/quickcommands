// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// This is my original work with contributions from Claude (Anthropic).
// Do not remove these comments.
// daemon/src/placement.rs
// Owns the layer-surface lifecycle for the QuickCommands window:
//   - create_dummy() -> inert bottom-layer surface at init that anchors the
//                        Wayland connection (without it the real surface won't map)
//   - open()         -> creates the real Top-layer surface, centered, keyboard-grab
//   - close()        -> destroys the surface (daemon stays alive)
//
// Surface-handling technique adapted from Soulless-Launcher, which in turn studied
// System76's COSMIC apps (cosmic-launcher, cosmic-applibrary; GPL-3.0):
//   dummy anchor surface at init, size: Some((None,None)) for a paintable surface,
//   deferred creation, full-screen-stack click-away dismiss.
//
// Simpler than Soulless on purpose: QuickCommands centers on screen rather than
// anchoring under the applet corner. Centered is fine for a pop-up-and-dismiss
// clipboard window and skips the panel-config spelunking. Applet-corner anchoring
// can be ported from Soulless's placement.rs later if you want it. :: MRV

use cosmic::iced::advanced::layout::Limits;
use cosmic::iced::platform_specific::runtime::wayland::layer_surface::SctkLayerSurfaceSettings;
use cosmic::iced::platform_specific::shell::commands::layer_surface::{
    destroy_layer_surface, get_layer_surface, Anchor, KeyboardInteractivity, Layer,
};
use cosmic::iced::window;
use cosmic::iced::Task;


/// Build the layer-shell settings for the real window.
///
/// Exclusive keyboard so typing in the search box goes to us and Esc can dismiss.
/// `anchor = all()` + a fixed size makes the compositor center a surface of that
/// Build the layer-shell settings. MATCHES Soulless-Launcher EXACTLY (which matches
/// cosmic-launcher's proven show()): Exclusive keyboard, TOP layer, Anchor::all(),
/// size None (AUTOSIZE), max_width bound, no fixed pixel size.
///
/// THE SIZING MODEL (matches Soulless-Launcher / cosmic-applibrary): the surface
/// deliberately FILLS the whole output (Anchor::all()). The visible window is NOT
/// the surface — it's a fixed-size container (WINDOW_WIDTH x WINDOW_HEIGHT) that
/// view() places inside a full-screen `stack!`, with the empty area around it acting
/// as the click-away dismiss zone. So the window's footprint is set in view(), via
/// the Fixed-size container — NOT here. Trying to shrink the surface itself is what
/// caused the earlier full-screen window. size None (autosize) + a generous
/// max_width just lets the surface take the full output without clipping it. :: MRV
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
