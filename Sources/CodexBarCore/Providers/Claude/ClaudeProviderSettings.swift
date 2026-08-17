import Foundation

public struct ClaudeProviderSettings: Sendable {
    public let usageDataSource: ClaudeUsageDataSource
    public let webExtrasEnabled: Bool
    /// Opt-in Claude statusLine observation feed. Off unless the user turns it on.
    public let statusLineFeedEnabled: Bool
    /// Mirrors the user's global Disable Keychain access preference, not the test-process safety gate.
    public let keychainAccessDisabled: Bool
    /// True only for the anonymous ambient Auto card. Explicit credentials and multi-account presentations
    /// cannot consume an identity-free observation.
    public let statusLineStandaloneAllowed: Bool
    public let cookieSource: ProviderCookieSource
    public let manualCookieHeader: String?
    public let organizationID: String?

    public init(
        usageDataSource: ClaudeUsageDataSource,
        webExtrasEnabled: Bool,
        statusLineFeedEnabled: Bool = false,
        keychainAccessDisabled: Bool = false,
        statusLineStandaloneAllowed: Bool = false,
        cookieSource: ProviderCookieSource,
        manualCookieHeader: String?,
        organizationID: String? = nil)
    {
        self.usageDataSource = usageDataSource
        self.webExtrasEnabled = webExtrasEnabled
        self.statusLineFeedEnabled = statusLineFeedEnabled
        self.keychainAccessDisabled = keychainAccessDisabled
        self.statusLineStandaloneAllowed = statusLineStandaloneAllowed
        self.cookieSource = cookieSource
        self.manualCookieHeader = manualCookieHeader
        self.organizationID = organizationID
    }
}

public enum ClaudeProviderSettingsKey: ProviderSettingsSectionKey {
    public static let providerID = ProviderInstanceID.claude
    public typealias Section = ClaudeProviderSettings
}

extension ProviderSettingsSnapshot {
    public typealias ClaudeProviderSettings = CodexBarCore.ClaudeProviderSettings
    public var claude: ClaudeProviderSettings? {
        self[ClaudeProviderSettingsKey.self]
    }

    public static func make(claude: ClaudeProviderSettings?) -> Self {
        self.make(claude, for: ClaudeProviderSettingsKey.self)
    }
}

extension ProviderSettingsSnapshotContribution {
    public static func claude(_ section: ClaudeProviderSettings) -> Self {
        Self(section, for: ClaudeProviderSettingsKey.self)
    }
}
