#!/usr/bin/env bash
set -euo pipefail

echo "==> Uninstalling TeleCode Menu Bar"

# Stop menu bar app
launchctl unload "$HOME/Library/LaunchAgents/com.telecode.bar.plist" 2>/dev/null || true
pkill -f "TeleCodeBar" 2>/dev/null || true

# Stop bot daemon
launchctl unload "$HOME/Library/LaunchAgents/com.telecode.bot.plist" 2>/dev/null || true

# Remove files
rm -rf "$HOME/Applications/TeleCodeBar.app"
rm -f "$HOME/Library/LaunchAgents/com.telecode.bar.plist"
rm -f "$HOME/Library/LaunchAgents/com.telecode.bot.plist"

echo "Done. Menu bar app and daemon plists removed."
