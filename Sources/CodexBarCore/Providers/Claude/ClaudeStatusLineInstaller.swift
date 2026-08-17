#if canImport(Darwin)
import Darwin
#elseif canImport(Glibc)
import Glibc
#elseif canImport(Musl)
import Musl
#endif
import Foundation

public enum ClaudeStatusLineInstallState: Equatable, Sendable {
    case absent
    case installed
    case needsRepair
    case userOwned
    case malformed
    case unsafeSymlink
}

public enum ClaudeStatusLineInstallerError: LocalizedError, Equatable {
    case helperUnavailable
    case userOwned
    case malformedSettings
    case unsafeSymlink
    case notInstalled

    public var errorDescription: String? {
        switch self {
        case .helperUnavailable:
            "CodexBarCLI was not found in the installed CodexBar app bundle."
        case .userOwned:
            "Claude Code already has a custom statusLine. CodexBar did not change it; use the manual composition guide."
        case .malformedSettings:
            "Claude Code settings are malformed. CodexBar did not overwrite them."
        case .unsafeSymlink:
            "Claude Code settings use a symbolic link. CodexBar refuses to write through it."
        case .notInstalled:
            "The Claude Code statusLine is not owned by CodexBar. Nothing was removed."
        }
    }
}

public enum ClaudeStatusLineInstaller {
    private static let statusLineKey = "statusLine"
    private static let commandType = "command"
    private static let commandSuffix = " claude statusline capture"

    public static func settingsURL(
        environment: [String: String] = ProcessInfo.processInfo.environment) -> URL
    {
        ClaudeConfigPaths.configRoot(environment: environment).appendingPathComponent("settings.json")
    }

    public static func bundledCLIURL(bundle: Bundle = .main) -> URL? {
        let url = bundle.bundleURL
            .appendingPathComponent("Contents", isDirectory: true)
            .appendingPathComponent("Helpers", isDirectory: true)
            .appendingPathComponent("CodexBarCLI", isDirectory: false)
        guard bundle.bundleURL.pathExtension == "app",
              FileManager.default.isExecutableFile(atPath: url.path)
        else { return nil }
        return url
    }

    public static func inspect(settingsURL: URL, executableURL: URL) -> ClaudeStatusLineInstallState {
        guard !self.isSymbolicLink(settingsURL.deletingLastPathComponent()),
              !self.isSymbolicLink(settingsURL)
        else { return .unsafeSymlink }
        guard FileManager.default.fileExists(atPath: settingsURL.path) else { return .absent }
        guard let root = self.readRoot(settingsURL) else { return .malformed }
        guard let raw = root[self.statusLineKey] else { return .absent }
        guard let object = raw as? [String: Any] else { return .userOwned }
        if self.isExactManagedObject(object, executableURL: executableURL) {
            return .installed
        }
        return self.isRecognizedManagedObject(object) ? .needsRepair : .userOwned
    }

    public static func install(settingsURL: URL, executableURL: URL) throws {
        try self.install(settingsURL: settingsURL, executableURL: executableURL, beforePublish: nil)
    }

    public static func uninstall(settingsURL: URL, executableURL: URL) throws {
        let state = self.inspect(settingsURL: settingsURL, executableURL: executableURL)
        guard state != .unsafeSymlink else { throw ClaudeStatusLineInstallerError.unsafeSymlink }
        guard state != .malformed else { throw ClaudeStatusLineInstallerError.malformedSettings }
        guard state == .installed || state == .needsRepair else {
            throw state == .userOwned ? ClaudeStatusLineInstallerError.userOwned : .notInstalled
        }
        guard var root = self.readRoot(settingsURL),
              let object = root[self.statusLineKey] as? [String: Any],
              self.isRecognizedManagedObject(object)
        else { throw ClaudeStatusLineInstallerError.notInstalled }
        root.removeValue(forKey: self.statusLineKey)
        try self.writeRoot(root, to: settingsURL, beforePublish: nil)
    }

    static func install(
        settingsURL: URL,
        executableURL: URL,
        beforePublish: ((URL) throws -> Void)?) throws
    {
        let state = self.inspect(settingsURL: settingsURL, executableURL: executableURL)
        switch state {
        case .installed:
            return
        case .userOwned:
            throw ClaudeStatusLineInstallerError.userOwned
        case .malformed:
            throw ClaudeStatusLineInstallerError.malformedSettings
        case .unsafeSymlink:
            throw ClaudeStatusLineInstallerError.unsafeSymlink
        case .absent, .needsRepair:
            break
        }
        var root = self.readRoot(settingsURL) ?? [:]
        root[self.statusLineKey] = self.managedObject(executableURL: executableURL)
        try self.writeRoot(root, to: settingsURL, beforePublish: beforePublish)
    }

    static func managedObject(executableURL: URL) -> [String: Any] {
        [
            "type": self.commandType,
            "command": self.shellQuote(executableURL.path) + self.commandSuffix,
        ]
    }

    private static func isExactManagedObject(_ object: [String: Any], executableURL: URL) -> Bool {
        guard object.count == 2,
              object["type"] as? String == self.commandType,
              object["command"] as? String == self.managedObject(executableURL: executableURL)["command"] as? String
        else { return false }
        return true
    }

    private static func isRecognizedManagedObject(_ object: [String: Any]) -> Bool {
        guard object.count == 2,
              object["type"] as? String == self.commandType,
              let command = object["command"] as? String,
              self.recognizedManagedExecutablePath(command: command) != nil
        else { return false }
        return true
    }

    /// Recognizes only the exact command grammar emitted by `managedObject`, while allowing the app path to move.
    private static func recognizedManagedExecutablePath(command: String) -> String? {
        guard command.hasSuffix(self.commandSuffix) else { return nil }
        let quotedPath = String(command.dropLast(self.commandSuffix.count))
        guard quotedPath.first == "'", quotedPath.last == "'", quotedPath.count >= 2 else { return nil }

        let encodedPath = String(quotedPath.dropFirst().dropLast())
        let path = encodedPath.replacingOccurrences(of: "'\"'\"'", with: "'")
        guard self.shellQuote(path) == quotedPath,
              path.hasPrefix("/"),
              !path.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains)
        else { return nil }

        let helperURL = URL(fileURLWithPath: path, isDirectory: false).standardizedFileURL
        guard helperURL.path == path,
              helperURL.lastPathComponent == "CodexBarCLI",
              helperURL.deletingLastPathComponent().lastPathComponent == "Helpers",
              helperURL.deletingLastPathComponent().deletingLastPathComponent().lastPathComponent == "Contents"
        else { return nil }
        let appURL = helperURL.deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        guard appURL.pathExtension.caseInsensitiveCompare("app") == .orderedSame,
              !appURL.deletingPathExtension().lastPathComponent.isEmpty
        else { return nil }
        return path
    }

    private static func readRoot(_ url: URL) -> [String: Any]? {
        guard let data = try? Data(contentsOf: url),
              let object = try? JSONSerialization.jsonObject(with: data),
              let root = object as? [String: Any]
        else { return nil }
        return root
    }

    private static func writeRoot(
        _ root: [String: Any],
        to url: URL,
        beforePublish: ((URL) throws -> Void)?) throws
    {
        let fileManager = FileManager.default
        let directory = url.deletingLastPathComponent()
        guard !self.isSymbolicLink(directory), !self.isSymbolicLink(url) else {
            throw ClaudeStatusLineInstallerError.unsafeSymlink
        }
        try fileManager.createDirectory(at: directory, withIntermediateDirectories: true)
        let mode = ((try? fileManager.attributesOfItem(atPath: url.path)[.posixPermissions]) as? NSNumber)?
            .uint16Value ?? 0o600
        let data = try JSONSerialization.data(withJSONObject: root, options: [.prettyPrinted, .sortedKeys])
        let staged = directory.appendingPathComponent(".settings.json.codexbar-staged-\(UUID().uuidString)")
        let descriptor = staged.path.withCString {
            open($0, O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC, mode_t(mode))
        }
        guard descriptor >= 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
        let handle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
        var isOpen = true
        do {
            guard fchmod(descriptor, mode_t(mode)) == 0 else {
                throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
            }
            try handle.write(contentsOf: data)
            try handle.synchronize()
            try handle.close()
            isOpen = false
            try beforePublish?(staged)
            let result = staged.path.withCString { source in
                url.path.withCString { destination in rename(source, destination) }
            }
            guard result == 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
        } catch {
            if isOpen {
                try? handle.close()
            }
            try? fileManager.removeItem(at: staged)
            throw error
        }
    }

    private static func isSymbolicLink(_ url: URL) -> Bool {
        guard let type = try? FileManager.default.attributesOfItem(atPath: url.path)[.type] as? FileAttributeType
        else { return false }
        return type == .typeSymbolicLink
    }

    private static func shellQuote(_ value: String) -> String {
        "'" + value.replacingOccurrences(of: "'", with: "'\"'\"'") + "'"
    }
}
