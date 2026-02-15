#!/usr/bin/env bash
set -euo pipefail

# ─── Resolve paths ───────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BINARY_PATH="$PROJECT_DIR/target/release/nitro-agent"
APP_DIR="$HOME/Applications/NitroBar.app"
CONTENTS_DIR="$APP_DIR/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"
PLIST_SOURCE="$SCRIPT_DIR/com.nitroagent.bot.plist"

echo "==> NitroAgent Menu Bar Installer"
echo "    Project: $PROJECT_DIR"
echo "    Binary:  $BINARY_PATH"
echo ""

# ─── Check binary exists ────────────────────────────────────────────
if [ ! -f "$BINARY_PATH" ]; then
    echo "Building release binary..."
    (cd "$PROJECT_DIR" && cargo build --release)
fi

# ─── Substitute paths in Swift source ────────────────────────────────
SWIFT_SRC="$SCRIPT_DIR/NitroBar.swift"
SWIFT_TMP="$SCRIPT_DIR/.NitroBar_resolved.swift"

sed \
    -e "s|__PROJECT_DIR__|$PROJECT_DIR|g" \
    -e "s|__BINARY_PATH__|$BINARY_PATH|g" \
    -e "s|__PLIST_SOURCE__|$PLIST_SOURCE|g" \
    -e "s|__HOME_DIR__|$HOME|g" \
    "$SWIFT_SRC" > "$SWIFT_TMP"

# ─── Compile ─────────────────────────────────────────────────────────
echo "==> Compiling NitroBar..."
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"

swiftc \
    -O \
    -o "$MACOS_DIR/NitroBar" \
    -framework AppKit \
    -target arm64-apple-macosx13.0 \
    "$SWIFT_TMP"

rm -f "$SWIFT_TMP"
echo "    Compiled: $MACOS_DIR/NitroBar"

# ─── Info.plist for the .app bundle ──────────────────────────────────
cat > "$CONTENTS_DIR/Info.plist" << 'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>com.nitroagent.bar</string>
    <key>CFBundleName</key>
    <string>NitroBar</string>
    <key>CFBundleExecutable</key>
    <string>NitroBar</string>
    <key>CFBundleVersion</key>
    <string>1.0</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>LSBackgroundOnly</key>
    <false/>
    <key>LSUIElement</key>
    <true/>
</dict>
</plist>
PLIST

echo "    App bundle: $APP_DIR"

# ─── Auto-start menu bar app on login ────────────────────────────────
MENUBAR_PLIST_LABEL="com.nitroagent.bar"
MENUBAR_PLIST_DEST="$HOME/Library/LaunchAgents/$MENUBAR_PLIST_LABEL.plist"

# Unload existing if present
launchctl unload "$MENUBAR_PLIST_DEST" 2>/dev/null || true

cat > "$MENUBAR_PLIST_DEST" << MPLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>$MENUBAR_PLIST_LABEL</string>
    <key>ProgramArguments</key>
    <array>
        <string>$MACOS_DIR/NitroBar</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <false/>
</dict>
</plist>
MPLIST

launchctl load "$MENUBAR_PLIST_DEST"
echo "    Menu bar app auto-starts on login"

# ─── Launch now (launchctl already started it, no need for open) ─────
echo ""
echo "==> NitroBar launched via launchctl"

echo ""
echo "Done! Look for 'TC' in your menu bar (top-right)."
echo ""
echo "From the menu bar you can:"
echo "  - Start/Stop the bot"
echo "  - Restart the bot"
echo "  - Enable/Disable auto-start on login"
echo "  - View logs"
echo "  - Open project folder"
