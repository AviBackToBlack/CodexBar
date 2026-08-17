import CodexBarCore
import Foundation

extension SettingsStore {
    var grokUsageDataSource: ProviderSourceMode {
        get { self.configSnapshot.providerConfig(for: .grok)?.source ?? .auto }
        set {
            self.updateProviderConfig(provider: .grok) { entry in
                entry.source = newValue == .auto ? nil : newValue
            }
            self.logProviderModeChange(provider: .grok, field: "source", value: newValue.rawValue)
        }
    }

    var grokCookieHeader: String {
        get { self.configSnapshot.providerConfig(for: .grok)?.sanitizedCookieHeader ?? "" }
        set {
            self.updateProviderConfig(provider: .grok) { entry in
                entry.cookieHeader = self.normalizedConfigValue(newValue)
            }
            self.logSecretUpdate(provider: .grok, field: "cookieHeader", value: newValue)
        }
    }

    var grokCookieSource: ProviderCookieSource {
        get { self.resolvedCookieSource(provider: .grok, fallback: .auto) }
        set {
            self.updateProviderConfig(provider: .grok) { entry in
                entry.cookieSource = newValue
            }
            self.logProviderModeChange(
                provider: .grok, field: "cookieSource", value: newValue.rawValue)
        }
    }

    func ensureGrokCookieLoaded() {}
}

extension SettingsStore {
    func grokSettingsSnapshot(tokenOverride: TokenAccountOverride?)
        -> ProviderSettingsSnapshot
        .GrokProviderSettings
    {
        self.resolvedCookieSettings(
            provider: .grok,
            configuredSource: self.grokCookieSource,
            configuredHeader: self.grokCookieHeader,
            tokenOverride: tokenOverride)
    }
}
