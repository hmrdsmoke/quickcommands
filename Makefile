# GPL-3.0-or-later - see LICENSE file for full terms
# Copyright 2026 Michael Van Auker (HMRDSmoke)
# This is my original work with contributions from Claude (Anthropic).
# Do not remove these comments.
#
# Install layout mirrors Soulless-Launcher:
#   - both binaries -> /usr/local/bin
#   - applet .desktop (X-CosmicApplet=true) -> /usr/share/applications  (panel picker reads this)
#   - daemon autostart entry -> /etc/xdg/autostart  (daemon is resident from login)
#   - a scalable SVG icon -> /usr/share/icons/hicolor/scalable/apps
#   - a `qc` helper that toggles the window (bind Super+V to it, or just run `qc`)
#
# Usage:
#   make                 # build (dev-release)
#   make release         # build (release, fully optimized)
#   make install         # build release + install everything, in one step
#   sudo make install-only   # install an ALREADY-BUILT release (run after `make release`)
#   sudo make uninstall  # remove everything
#
# NOTE on sudo + rustup: `sudo make install` runs cargo as ROOT, which usually has
# no default Rust toolchain. If you hit "rustup could not choose a version", build
# as yourself and install as root in two steps:
#     make release          # as your user — builds the binaries
#     sudo make install-only # as root — just copies files, no rebuild

PREFIX    ?= /usr/local
BINDIR    := $(PREFIX)/bin
APPDIR    := /usr/share/applications
AUTOSTART := /etc/xdg/autostart
ICON_BASE := /usr/share/icons/hicolor
ICON_SVG  := assets/icons/hicolor/scalable/apps/com.github.hmrdsmoke.quickcommands.svg

DAEMON_ID := com.github.hmrdsmoke.quickcommands
APPLET_ID := com.github.hmrdsmoke.quickcommands-applet

DAEMON_BIN := target/release/quickcommands
APPLET_BIN := target/release/quickcommands-applet

APPLET_DESK    := applet/resources/quickcommands-applet.desktop
AUTOSTART_DESK := assets/com.github.hmrdsmoke.quickcommands.autostart.desktop

# ── Build ──────────────────────────────────────────────────────────────────────
all:
	cargo build --workspace --profile dev-release

release:
	cargo build --workspace --release

# ── Install ────────────────────────────────────────────────────────────────────
# `install` builds first (use without sudo if your user owns /usr/local, else use
# the two-step: `make release` then `sudo make install-only`).
install: release install-only

# `install-only` copies an already-built release. Safe under sudo — never builds,
# so it doesn't need root to have a Rust toolchain.
install-only:
	@test -x $(DAEMON_BIN) || { echo "ERROR: $(DAEMON_BIN) not found. Run 'make release' first (as your user)."; exit 1; }
	@echo "Installing daemon  → $(BINDIR)/quickcommands"
	install -Dm755 $(DAEMON_BIN) $(BINDIR)/quickcommands
	@echo "Installing applet  → $(BINDIR)/quickcommands-applet"
	install -Dm755 $(APPLET_BIN) $(BINDIR)/quickcommands-applet
	@echo "Installing applet desktop entry → $(APPDIR)/$(APPLET_ID).desktop"
	install -Dm644 $(APPLET_DESK) $(APPDIR)/$(APPLET_ID).desktop
	@echo "Installing daemon autostart entry → $(AUTOSTART)"
	install -Dm644 $(AUTOSTART_DESK) $(AUTOSTART)/$(DAEMON_ID).desktop
	@echo "Installing scalable icon (daemon + applet share the art)"
	install -Dm644 $(ICON_SVG) $(ICON_BASE)/scalable/apps/$(DAEMON_ID).svg
	install -Dm644 $(ICON_SVG) $(ICON_BASE)/scalable/apps/$(APPLET_ID).svg
	@echo "Installing 'qc' toggle helper → $(BINDIR)/qc"
	printf '#!/bin/sh\nexec /usr/local/bin/quickcommands toggle\n' > /tmp/qc
	install -Dm755 /tmp/qc $(BINDIR)/qc
	@echo ""
	@echo "Done. Log out/in (or run: /usr/local/bin/quickcommands &) to start the daemon,"
	@echo "then add the 'QuickCommands Applet' to your COSMIC panel. Bind Super+V to 'qc'."

# ── Uninstall ──────────────────────────────────────────────────────────────────
uninstall:
	rm -f $(BINDIR)/quickcommands
	rm -f $(BINDIR)/quickcommands-applet
	rm -f $(BINDIR)/qc
	rm -f $(APPDIR)/$(APPLET_ID).desktop
	rm -f $(AUTOSTART)/$(DAEMON_ID).desktop
	rm -f $(ICON_BASE)/scalable/apps/$(DAEMON_ID).svg
	rm -f $(ICON_BASE)/scalable/apps/$(APPLET_ID).svg
	@echo "Uninstalled. (Your data in ~/.local/share/quickcommands is untouched.)"

.PHONY: all release install install-only uninstall
