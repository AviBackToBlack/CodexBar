import CoreFoundation
import Crypto
import Foundation

public enum ClaudeStatusLineFeed {
    public static let standaloneSourceLabel = "statusline-standalone"
    public static let schemaVersion = 1
    public static let maximumInputBytes = 1_048_576
    /// StatusLine reset epochs outside 2000-01-01 through 2100-01-01 are treated as drifted metadata.
    public static let validResetEpochSecondsRange: ClosedRange<Double> = 946_684_800...4_102_444_800
}

public struct ClaudeStatusLineWindow: Codable, Equatable, Sendable {
    public let usedPercent: Double
    public let resetsAt: Date?

    public init(usedPercent: Double, resetsAt: Date?) {
        self.usedPercent = usedPercent
        self.resetsAt = resetsAt
    }
}

public struct ClaudeStatusLineRateLimits: Codable, Equatable, Sendable {
    public let profileID: String
    public let capturedAt: Date
    public let fiveHour: ClaudeStatusLineWindow?
    public let sevenDay: ClaudeStatusLineWindow?

    public init(
        profileID: String,
        capturedAt: Date,
        fiveHour: ClaudeStatusLineWindow?,
        sevenDay: ClaudeStatusLineWindow?)
    {
        self.profileID = profileID
        self.capturedAt = capturedAt
        self.fiveHour = fiveHour
        self.sevenDay = sevenDay
    }
}

public enum ClaudeStatusLineProfile {
    public static func identifier(environment: [String: String]) -> String {
        let raw = environment[ClaudeConfigPaths.configDirectoryEnvironmentKey] ?? ""
        let material = raw.isEmpty ? "codexbar.claude-statusline.default" : "codexbar.claude-statusline.\(raw)"
        return SHA256.hash(data: Data(material.utf8)).map { String(format: "%02x", $0) }.joined()
    }
}

/// Parses Claude Code's statusLine stdin. Only the two documented rate-limit windows are accepted.
public enum ClaudeStatusLinePayloadParser {
    public static func parseOfficialPayload(
        _ data: Data,
        capturedAt: Date = Date(),
        environment: [String: String] = ProcessInfo.processInfo.environment) -> ClaudeStatusLineRateLimits?
    {
        guard data.count <= ClaudeStatusLineFeed.maximumInputBytes,
              let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let rateLimits = root["rate_limits"] as? [String: Any]
        else { return nil }

        let fiveHour = self.window(rateLimits["five_hour"])
        let sevenDay = self.window(rateLimits["seven_day"])
        guard fiveHour != nil || sevenDay != nil else { return nil }
        return ClaudeStatusLineRateLimits(
            profileID: ClaudeStatusLineProfile.identifier(environment: environment),
            capturedAt: capturedAt,
            fiveHour: fiveHour,
            sevenDay: sevenDay)
    }

    private static func window(_ raw: Any?) -> ClaudeStatusLineWindow? {
        guard let object = raw as? [String: Any],
              let used = self.finiteNumber(object["used_percentage"]),
              (0...100).contains(used)
        else { return nil }
        return ClaudeStatusLineWindow(usedPercent: used, resetsAt: self.resetDate(object["resets_at"]))
    }

    private static func resetDate(_ raw: Any?) -> Date? {
        guard let seconds = self.finiteNumber(raw),
              ClaudeStatusLineFeed.validResetEpochSecondsRange.contains(seconds)
        else { return nil }
        return Date(timeIntervalSince1970: seconds)
    }

    private static func finiteNumber(_ raw: Any?) -> Double? {
        guard let number = raw as? NSNumber,
              CFGetTypeID(number) != CFBooleanGetTypeID()
        else { return nil }
        let value = number.doubleValue
        return value.isFinite ? value : nil
    }
}

struct ClaudeStatusLineObservationEnvelope: Codable, Equatable {
    let schema: Int
    let observation: ClaudeStatusLineRateLimits
}
