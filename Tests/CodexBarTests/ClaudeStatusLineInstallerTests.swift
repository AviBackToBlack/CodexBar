import Foundation
import Testing
@testable import CodexBarCore

@Suite(.serialized)
struct ClaudeStatusLineInstallerTests {
    private func fixture() throws -> (root: URL, settings: URL, executable: URL) {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("codexbar-statusline-installer-\(UUID().uuidString)", isDirectory: true)
        let settings = root.appendingPathComponent(".claude/settings.json")
        let executable = root.appendingPathComponent("CodexBar.app/Contents/Helpers/CodexBarCLI")
        try FileManager.default.createDirectory(
            at: executable.deletingLastPathComponent(),
            withIntermediateDirectories: true)
        try Data().write(to: executable)
        return (root, settings, executable)
    }

    private func rootObject(_ url: URL) throws -> [String: Any] {
        let data = try Data(contentsOf: url)
        return try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    @Test
    func `absent settings install exact managed object and preserve unrelated JSON`() throws {
        let fixture = try self.fixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        try FileManager.default.createDirectory(
            at: fixture.settings.deletingLastPathComponent(),
            withIntermediateDirectories: true)
        try Data(#"{"theme":"dark"}"#.utf8).write(to: fixture.settings)
        try FileManager.default.setAttributes([.posixPermissions: 0o640], ofItemAtPath: fixture.settings.path)

        #expect(ClaudeStatusLineInstaller.inspect(
            settingsURL: fixture.settings,
            executableURL: fixture.executable) == .absent)
        try ClaudeStatusLineInstaller.install(settingsURL: fixture.settings, executableURL: fixture.executable)

        let root = try self.rootObject(fixture.settings)
        let statusLine = try #require(root["statusLine"] as? [String: Any])
        #expect(root["theme"] as? String == "dark")
        #expect(statusLine.count == 2)
        #expect(statusLine["type"] as? String == "command")
        #expect((statusLine["command"] as? String)?.hasSuffix(" claude statusline capture") == true)
        let mode = try #require(
            (FileManager.default.attributesOfItem(atPath: fixture.settings.path)[.posixPermissions] as? NSNumber)?
                .intValue)
        #expect(mode == 0o640)
        #expect(ClaudeStatusLineInstaller.inspect(
            settingsURL: fixture.settings,
            executableURL: fixture.executable) == .installed)
    }

    @Test
    func `atomic install leaves old settings visible until publish`() throws {
        let fixture = try self.fixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        try FileManager.default.createDirectory(
            at: fixture.settings.deletingLastPathComponent(),
            withIntermediateDirectories: true)
        let original = Data(#"{"theme":"light"}"#.utf8)
        try original.write(to: fixture.settings)

        try ClaudeStatusLineInstaller.install(
            settingsURL: fixture.settings,
            executableURL: fixture.executable,
            beforePublish: { _ in
                let visible = try Data(contentsOf: fixture.settings)
                #expect(visible == original)
            })
        #expect(try self.rootObject(fixture.settings)["statusLine"] != nil)
    }

    @Test
    func `owned command at an old app path is repairable`() throws {
        let fixture = try self.fixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        try ClaudeStatusLineInstaller.install(settingsURL: fixture.settings, executableURL: fixture.executable)
        let moved = fixture.root.appendingPathComponent("Moved.app/Contents/Helpers/CodexBarCLI")
        #expect(ClaudeStatusLineInstaller.inspect(
            settingsURL: fixture.settings,
            executableURL: moved) == .needsRepair)
        try ClaudeStatusLineInstaller.install(settingsURL: fixture.settings, executableURL: moved)
        #expect(ClaudeStatusLineInstaller.inspect(
            settingsURL: fixture.settings,
            executableURL: moved) == .installed)
    }

    @Test
    func `prefixed injected and composed commands remain user owned`() throws {
        let fixture = try self.fixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        try FileManager.default.createDirectory(
            at: fixture.settings.deletingLastPathComponent(),
            withIntermediateDirectories: true)
        let cleanCommand = "'\(fixture.executable.path)' claude statusline capture"
        let nonAppHelper = fixture.root.appendingPathComponent("NotAnApp/Contents/Helpers/CodexBarCLI").path
        let commands = [
            "/usr/bin/env \(cleanCommand)",
            "echo injected; \(cleanCommand)",
            "input=$(cat); printf '%s' \"$input\" | \(cleanCommand)",
            "'\(nonAppHelper)' claude statusline capture",
        ]

        for command in commands {
            let original = try JSONSerialization.data(withJSONObject: [
                "theme": "dark",
                "statusLine": ["type": "command", "command": command],
            ], options: [.sortedKeys])
            try original.write(to: fixture.settings)

            #expect(ClaudeStatusLineInstaller.inspect(
                settingsURL: fixture.settings,
                executableURL: fixture.executable) == .userOwned)
            #expect(throws: ClaudeStatusLineInstallerError.userOwned) {
                try ClaudeStatusLineInstaller.install(
                    settingsURL: fixture.settings,
                    executableURL: fixture.executable)
            }
            #expect(try Data(contentsOf: fixture.settings) == original)
            #expect(throws: ClaudeStatusLineInstallerError.userOwned) {
                try ClaudeStatusLineInstaller.uninstall(
                    settingsURL: fixture.settings,
                    executableURL: fixture.executable)
            }
            #expect(try Data(contentsOf: fixture.settings) == original)
        }
    }

    @Test
    func `user owned and malformed settings are never overwritten`() throws {
        for contents in [
            #"{"statusLine":{"type":"command","command":"~/mine.sh"},"theme":"dark"}"#,
            #"["unexpected-root"]"#,
            "{not-json",
        ] {
            let fixture = try self.fixture()
            defer { try? FileManager.default.removeItem(at: fixture.root) }
            try FileManager.default.createDirectory(
                at: fixture.settings.deletingLastPathComponent(),
                withIntermediateDirectories: true)
            let original = Data(contents.utf8)
            try original.write(to: fixture.settings)
            #expect(throws: (any Error).self) {
                try ClaudeStatusLineInstaller.install(
                    settingsURL: fixture.settings,
                    executableURL: fixture.executable)
            }
            #expect(try Data(contentsOf: fixture.settings) == original)
        }
    }

    @Test
    func `installer rejects settings symlinks`() throws {
        let fixture = try self.fixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        try FileManager.default.createDirectory(
            at: fixture.settings.deletingLastPathComponent(),
            withIntermediateDirectories: true)
        let target = fixture.root.appendingPathComponent("target.json")
        try Data(#"{}"#.utf8).write(to: target)
        try FileManager.default.createSymbolicLink(at: fixture.settings, withDestinationURL: target)
        #expect(ClaudeStatusLineInstaller.inspect(
            settingsURL: fixture.settings,
            executableURL: fixture.executable) == .unsafeSymlink)
        #expect(throws: ClaudeStatusLineInstallerError.unsafeSymlink) {
            try ClaudeStatusLineInstaller.install(settingsURL: fixture.settings, executableURL: fixture.executable)
        }
    }

    @Test
    func `uninstall removes only an exact CodexBar owned object`() throws {
        let fixture = try self.fixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        try ClaudeStatusLineInstaller.install(settingsURL: fixture.settings, executableURL: fixture.executable)
        var root = try self.rootObject(fixture.settings)
        root["theme"] = "dark"
        try JSONSerialization.data(withJSONObject: root).write(to: fixture.settings)
        try ClaudeStatusLineInstaller.uninstall(settingsURL: fixture.settings, executableURL: fixture.executable)
        let after = try self.rootObject(fixture.settings)
        #expect(after["statusLine"] == nil)
        #expect(after["theme"] as? String == "dark")

        try Data(#"{"statusLine":{"type":"command","command":"~/mine.sh"}}"#.utf8)
            .write(to: fixture.settings)
        let original = try Data(contentsOf: fixture.settings)
        #expect(throws: ClaudeStatusLineInstallerError.userOwned) {
            try ClaudeStatusLineInstaller.uninstall(settingsURL: fixture.settings, executableURL: fixture.executable)
        }
        #expect(try Data(contentsOf: fixture.settings) == original)
    }
}
