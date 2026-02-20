#!/usr/bin/env bash
set -euo pipefail

# ─── Headless launchd service installer ─────────────────────────────
# Installs NitroAgent as a background macOS service via launchd.
# The bot starts automatically on login and restarts on crash.
# No menu bar app, no GUI — just the daemon.
#
# Usage:
#   bash install-service.sh          # install and start
#   bash install-service.sh --status # check if running

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BINARY_PATH="$SCRIPT_DIR/target/release/nitro-agent"
LABEL="com.nitroagent.bot"
PLIST_DEST="$HOME/Library/LaunchAgents/$LABEL.plist"
LOG_DIR="$SCRIPT_DIR/logs"

# ─── Status check ───────────────────────────────────────────────────
if [[ "${1:-}" == "--status" ]]; then
    if pgrep -f "nitro-agent" > /dev/null 2>&1; then
        PID=$(pgrep -f "target/release/nitro-agent" | head -1)
        echo "✅ NitroAgent is running (PID $PID)"
    else
        echo "⏹  NitroAgent is not running"
    fi
    exit 0
fi

echo "==> NitroAgent Service Installer"
echo "    Project: $SCRIPT_DIR"
echo ""

# ─── Build if needed ────────────────────────────────────────────────
if [ ! -f "$BINARY_PATH" ]; then
    echo "==> Building release binary..."
    (cd "$SCRIPT_DIR" && cargo build --release)
    echo ""
fi

# ─── Ensure log directory exists ─────────────────────────────────────
mkdir -p "$LOG_DIR"

# ─── Unload existing service if present ──────────────────────────────
if launchctl list "$LABEL" > /dev/null 2>&1; then
    echo "==> Stopping existing service..."
    launchctl unload "$PLIST_DEST" 2>/dev/null || true
fi

# ─── Resolve PATH for Claude CLI discovery ───────────────────────────
RESOLVED_PATH="$HOME/.local/bin:$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin"

# ─── Generate plist ──────────────────────────────────────────────────
cat > "$PLIST_DEST" << PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>$LABEL</string>

    <key>ProgramArguments</key>
    <array>
        <string>$BINARY_PATH</string>
    </array>

    <key>WorkingDirectory</key>
    <string>$SCRIPT_DIR</string>

    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>$RESOLVED_PATH</string>
    </dict>

    <key>RunAtLoad</key>
    <true/>

    <key>KeepAlive</key>
    <true/>

    <key>StandardOutPath</key>
    <string>$LOG_DIR/daemon-stdout.log</string>

    <key>StandardErrorPath</key>
    <string>$LOG_DIR/daemon-stderr.log</string>

    <key>ThrottleInterval</key>
    <integer>5</integer>
</dict>
</plist>
PLIST

echo "==> Service plist written to $PLIST_DEST"

# ─── Load and start ─────────────────────────────────────────────────
launchctl load "$PLIST_DEST"

echo "==> Service loaded"
echo ""

# ─── Verify ──────────────────────────────────────────────────────────
sleep 2
if pgrep -f "target/release/nitro-agent" > /dev/null 2>&1; then
    PID=$(pgrep -f "target/release/nitro-agent" | head -1)
    echo "✅ NitroAgent is running (PID $PID)"
else
    echo "⚠️  NitroAgent may still be starting. Check logs:"
    echo "    tail -f $LOG_DIR/daemon-stderr.log"
fi

echo ""
echo "The bot will:"
echo "  - Start automatically on login"
echo "  - Restart automatically on crash"
echo "  - Log to $LOG_DIR/daemon-{stdout,stderr}.log"
echo ""
echo "To stop:   bash $(basename "$0" .sh)-uninstall.sh"
echo "To status: bash $(basename "$0") --status"
