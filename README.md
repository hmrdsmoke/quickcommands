# QuickCommands

A clipboard manager with saved **QuickCommands** for the COSMIC desktop.

Pop up a window, see your recent copies and your saved command snippets (git,
cargo, per-project), click any one to copy it to the clipboard, done. Built for
COSMIC / Pop!_OS in Rust with libcosmic/iced.

## What it does

- **Recent** — a rolling history of what you've copied (last 100), newest first.
  Click an entry to copy it back to the clipboard.
- **QuickCommands sections** — named groups ("Git", "Cargo", your projects) of
  saved commands. Click one to copy its command text, ready to paste in a
  terminal. Collapsible; the collapsed state is remembered.
- **Filter box** — type to narrow both the history and the saved commands at once.
- **Pop-up-and-dismiss** — the window is a layer surface: it floats above
  everything, grabs the keyboard, and dismisses on Esc or click-away. Clicking an
  item copies it and closes the window.

## Architecture

A **resident daemon + thin applet**, the same shape as Soulless-Launcher
(native-first; no Flatpak privileged-socket machinery yet).

```
quickcommands/
├── core/     quickcommands-core   — storage models + clipboard engine (no UI)
├── daemon/   quickcommands        — the resident window daemon (layer surface)
└── applet/   quickcommands-applet — panel button that pokes the daemon
```

- **daemon** (`quickcommands`): runs windowless and resident via
  `run_single_instance` + `no_main_window`, kept alive by a parked subscription.
  A kernel flock in `$XDG_RUNTIME_DIR` referees single-instance. It owns the
  layer-surface window and shows/hides it on D-Bus activation instead of
  cold-starting. A background thread watches the clipboard and records copies.
- **applet** (`quickcommands-applet`): an icon button. On press it sends
  `ActivateAction("toggle")` to the daemon over the session bus (in-process, no
  child spawned per click). If no daemon is running, it spawns one.
- **core** (`quickcommands-core`): the shared data layer — `History`,
  `Sections`, and the `clipboard` module (read/write/watch via `wl-clipboard-rs`
  on `wlr-data-control`). Polled at 500ms; swap to the event-driven `watch`
  module later if desired.

### How it's triggered

Two ways into the same window, both hitting the daemon's toggle:

1. **Panel applet** — click the QuickCommands icon.
2. **Keyboard shortcut** — bind a COSMIC custom shortcut (e.g. `Super+V`) to
   `quickcommands toggle`.

(The COSMIC *desktop* right-click menu is not an extension point as of Oct 2026
— it belongs to `cosmic-comp` and would require forking the compositor. Applet +
hotkey give the same "appears where my hand is" feel without that.)

## Storage

Everything lives in `~/.local/share/quickcommands/`:

- `history.json` — the rolling clipboard history (churns constantly)
- `sections.json` — your saved QuickCommands (hand-curated)

Both are written atomically (temp + rename). A corrupt file is logged and treated
as empty rather than crashing the daemon. On first run `sections.json` is seeded
with a starter Git + Cargo set you can edit freely.

See [`sections.example.json`](sections.example.json) for the format. The window
shows three columns: **Recent** (clipboard history), **Git** (the section named
`Git`), and **Projects** (every other section). Commands are copied on click, so a
`cd ~/Projects/foo && make release` entry pastes ready to run. Edit `sections.json`
and reopen the window to pick up changes.

## Build

```sh
cargo build --workspace --profile dev-release   # day-to-day
cargo build --workspace --release               # shipping
```

Needs the usual COSMIC build deps (libxkbcommon, wayland, etc.) — same as any
libcosmic app.

## Run (native)

```sh
# Start the daemon (normally autostarted at login):
quickcommands &

# Toggle the window from anywhere (bind this to Super+V in COSMIC settings):
quickcommands toggle
```

Install the applet's desktop entry so the panel can add it:

```sh
install -Dm644 applet/resources/quickcommands-applet.desktop \
  ~/.local/share/applications/quickcommands-applet.desktop
```

Then add **QuickCommands Applet** to the panel in COSMIC Settings.

## Status / next

Working skeleton — compiles clean against libcosmic
`fc5debccef41cd6bd6731cbc768cb719847d1a2f` (same rev as Soulless). Still to do:

- Add/edit/delete commands and sections from the UI (currently edit
  `sections.json` by hand).
- Pin a history entry; delete a single entry; clear history.
- Applet-corner window anchoring (currently centered) — portable from Soulless's
  `placement.rs` if wanted.
- Flatpak packaging + the privileged-socket armor, if submitting to the Store.
- Optional: switch the clipboard watch from polling to the event-driven
  `wl-clipboard-rs` `watch::Watcher`.

## License

GPL-3.0-or-later. © 2026 Michael Van Auker (HMRDSmoke).
