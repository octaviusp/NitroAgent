import AppKit
import Foundation

// ─── Config ─────────────────────────────────────────────────────────
private let kLabel       = "com.nitroagent.bot"
private let kProjectDir  = "__PROJECT_DIR__"
private let kBinaryPath  = "__BINARY_PATH__"
private let kPlistSource = "__PLIST_SOURCE__"
private let kPlistDest   = NSHomeDirectory() + "/Library/LaunchAgents/\(kLabel).plist"
private let kLogPath     = kProjectDir + "/logs/daemon-stderr.log"

// ─── App Delegate ───────────────────────────────────────────────────

class AppDelegate: NSObject, NSApplicationDelegate {
    private var statusItem: NSStatusItem!
    private var timer: Timer?

    func applicationDidFinishLaunching(_ notification: Notification) {
        statusItem = NSStatusBar.system.statusItem(withLength: 28)
        if let btn = statusItem.button {
            btn.title = "\u{1F916}"  // robot face emoji
        }

        rebuildMenu()

        // Poll status every 5 seconds
        timer = Timer.scheduledTimer(withTimeInterval: 5.0, repeats: true) { [weak self] _ in
            self?.rebuildMenu()
        }
    }

    private func rebuildMenu() {
        let menu = NSMenu()
        let running = isRunning()

        // ── Status ──
        let statusTitle = running ? "● Running" : "○ Stopped"
        let statusItem = NSMenuItem(title: statusTitle, action: nil, keyEquivalent: "")
        statusItem.isEnabled = false
        if running, let pid = getPid() {
            statusItem.title = "● Running (PID \(pid))"
        }
        menu.addItem(statusItem)
        menu.addItem(NSMenuItem.separator())

        // ── Toggle ──
        if running {
            menu.addItem(NSMenuItem(title: "Stop Bot", action: #selector(stopBot), keyEquivalent: "s"))
        } else {
            menu.addItem(NSMenuItem(title: "Start Bot", action: #selector(startBot), keyEquivalent: "s"))
        }

        // ── Restart ──
        let restartItem = NSMenuItem(title: "Restart Bot", action: running ? #selector(restartBot) : nil, keyEquivalent: "r")
        menu.addItem(restartItem)

        menu.addItem(NSMenuItem.separator())

        // ── Auto-start ──
        let autoStartEnabled = FileManager.default.fileExists(atPath: kPlistDest)
        let autoItem = NSMenuItem(
            title: autoStartEnabled ? "Disable Auto-Start" : "Enable Auto-Start",
            action: #selector(toggleAutoStart),
            keyEquivalent: "a"
        )
        menu.addItem(autoItem)

        menu.addItem(NSMenuItem.separator())

        // ── Logs ──
        menu.addItem(NSMenuItem(title: "View Logs", action: #selector(viewLogs), keyEquivalent: "l"))
        menu.addItem(NSMenuItem(title: "Open Project Folder", action: #selector(openProject), keyEquivalent: "o"))

        menu.addItem(NSMenuItem.separator())
        menu.addItem(NSMenuItem(title: "Quit NitroAgent", action: #selector(quitApp), keyEquivalent: "q"))

        self.statusItem.menu = menu

        // Update icon based on status
        if let btn = self.statusItem.button {
            btn.title = running ? "\u{1F916}" : "\u{1F6D1}"  // robot vs stop sign
        }
    }

    // ─── Actions ────────────────────────────────────────────────────

    @objc private func startBot() {
        // Try launchctl first if plist installed
        if FileManager.default.fileExists(atPath: kPlistDest) {
            shell("launchctl", "load", kPlistDest)
        } else {
            // Direct launch
            let task = Process()
            task.executableURL = URL(fileURLWithPath: kBinaryPath)
            task.currentDirectoryURL = URL(fileURLWithPath: kProjectDir)
            task.environment = buildEnv()
            task.standardOutput = FileHandle.nullDevice
            task.standardError = FileHandle.nullDevice
            task.arguments = []
            try? task.run()
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { [weak self] in
            self?.rebuildMenu()
        }
    }

    @objc private func stopBot() {
        if FileManager.default.fileExists(atPath: kPlistDest) {
            shell("launchctl", "unload", kPlistDest)
        }
        // Also kill any direct instances
        if let pid = getPid() {
            shell("kill", String(pid))
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) { [weak self] in
            self?.rebuildMenu()
        }
    }

    @objc private func restartBot() {
        stopBot()
        DispatchQueue.main.asyncAfter(deadline: .now() + 2.0) { [weak self] in
            self?.startBot()
        }
    }

    @objc private func toggleAutoStart() {
        if FileManager.default.fileExists(atPath: kPlistDest) {
            // Disable: unload + remove plist
            shell("launchctl", "unload", kPlistDest)
            try? FileManager.default.removeItem(atPath: kPlistDest)
        } else {
            // Enable: copy plist + load
            installPlist()
            shell("launchctl", "load", kPlistDest)
        }
        rebuildMenu()
    }

    @objc private func viewLogs() {
        let logURL = URL(fileURLWithPath: kLogPath)
        if FileManager.default.fileExists(atPath: kLogPath) {
            NSWorkspace.shared.open(logURL)
        } else {
            let alert = NSAlert()
            alert.messageText = "No log file found"
            alert.informativeText = kLogPath
            alert.runModal()
        }
    }

    @objc private func openProject() {
        NSWorkspace.shared.open(URL(fileURLWithPath: kProjectDir))
    }

    @objc private func quitApp() {
        NSApplication.shared.terminate(nil)
    }

    // ─── Helpers ────────────────────────────────────────────────────

    private func isRunning() -> Bool {
        return getPid() != nil
    }

    private func getPid() -> Int32? {
        let pipe = Pipe()
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/usr/bin/pgrep")
        task.arguments = ["-f", "nitro-agent"]
        task.standardOutput = pipe
        task.standardError = FileHandle.nullDevice
        try? task.run()
        task.waitUntilExit()

        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        guard let output = String(data: data, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines),
              !output.isEmpty else {
            return nil
        }
        // First PID (exclude our own pgrep)
        let pids = output.components(separatedBy: "\n").compactMap { Int32($0) }
        return pids.first
    }

    private func buildEnv() -> [String: String] {
        // Load .env from project dir
        var env = ProcessInfo.processInfo.environment
        let home = NSHomeDirectory()
        env["PATH"] = "\(home)/.local/bin:\(home)/.bun/bin:/usr/local/bin:/usr/bin:/bin:/opt/homebrew/bin:" + (env["PATH"] ?? "")

        let dotEnvPath = kProjectDir + "/.env"
        if let contents = try? String(contentsOfFile: dotEnvPath, encoding: .utf8) {
            for line in contents.components(separatedBy: "\n") {
                let trimmed = line.trimmingCharacters(in: .whitespaces)
                if trimmed.isEmpty || trimmed.hasPrefix("#") { continue }
                let parts = trimmed.split(separator: "=", maxSplits: 1)
                if parts.count == 2 {
                    let key = String(parts[0]).trimmingCharacters(in: .whitespaces)
                    let val = String(parts[1]).trimmingCharacters(in: .whitespaces)
                    env[key] = val
                }
            }
        }
        return env
    }

    private func installPlist() {
        // Read template plist, substitute paths, write to LaunchAgents
        guard let template = try? String(contentsOfFile: kPlistSource, encoding: .utf8) else {
            return
        }
        let resolved = template
            .replacingOccurrences(of: "__BINARY_PATH__", with: kBinaryPath)
            .replacingOccurrences(of: "__PROJECT_DIR__", with: kProjectDir)
            .replacingOccurrences(of: "__HOME_DIR__", with: NSHomeDirectory())

        let destDir = NSHomeDirectory() + "/Library/LaunchAgents"
        try? FileManager.default.createDirectory(atPath: destDir, withIntermediateDirectories: true)
        try? resolved.write(toFile: kPlistDest, atomically: true, encoding: .utf8)
    }

    @discardableResult
    private func shell(_ args: String...) -> Int32 {
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        task.arguments = args
        task.standardOutput = FileHandle.nullDevice
        task.standardError = FileHandle.nullDevice
        try? task.run()
        task.waitUntilExit()
        return task.terminationStatus
    }
}

// ─── Main ───────────────────────────────────────────────────────────

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.accessory) // No Dock icon, menu bar only
app.run()
