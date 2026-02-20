#!/usr/bin/env bash
set -euo pipefail

# ─── Headless launchd service uninstaller ────────────────────────────
# Stops NitroAgent and removes the launchd service.
# Does NOT delete the binary, config, or data — only the daemon setup.

LABEL="com.nitroagent.bot"
PLIST_DEST="$HOME/Library/LaunchAgents/$LABEL.plist"

echo "==> NitroAgent Service Uninstaller"
echo ""

# ─── Stop and unload ────────────────────────────────────────────────
if launchctl list "$LABEL" > /dev/null 2>&1; then
    echo "==> Stopping service..."
    launchctl unload "$PLIST_DEST" 2>/dev/null || true
    echo "    Service stopped"
else
    echo "    Service not loaded"
fi

# ─── Remove plist ───────────────────────────────────────────────────
if [ -f "$PLIST_DEST" ]; then
    rm "$PLIST_DEST"
    echo "    Removed $PLIST_DEST"
else
    echo "    No plist found"
fi

echo ""
echo "✅ NitroAgent service removed."
echo "   Binary, config, and data are untouched."
echo "   To reinstall: bash install-service.sh"
