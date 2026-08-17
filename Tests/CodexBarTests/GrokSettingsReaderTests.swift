import Foundation
import Testing
@testable import CodexBarCore

struct GrokSettingsReaderTests {
    @Test
    func `normalizes a pasted SuperGrok bearer and rejects cookies`() {
        #expect(GrokSettingsReader.normalizedOAuthToken("  Bearer abc.def.ghi  ") == "abc.def.ghi")
        #expect(GrokSettingsReader.normalizedOAuthToken("Cookie: sso=abc") == nil)
        #expect(GrokSettingsReader.normalizedOAuthToken("sso=abc; sso-rw=def") == nil)
        #expect(GrokSettingsReader.normalizedOAuthToken("xai-mgmt-key") == nil)
        #expect(GrokSettingsReader.normalizedOAuthToken("   ") == nil)
    }

    @Test
    func `reads pasted SuperGrok credentials from GROK_OAUTH_TOKEN`() {
        let env = [GrokSettingsReader.oauthTokenEnvironmentKey: "Bearer pasted-token"]
        let creds = GrokSettingsReader.pastedCredentials(environment: env)

        #expect(creds?.accessToken == "pasted-token")
        #expect(creds?.loginMethod == "SuperGrok")
        #expect(GrokSettingsReader.oauthAccessToken(environment: [:]) == nil)
    }

    @Test
    func `descriptor exposes SuperGrok token accounts`() {
        let support = GrokProviderDescriptor.descriptor.credentials?.tokenAccountSupport
        #expect(support?.title == "SuperGrok tokens")
        if case let .environment(key) = support?.injection {
            #expect(key == GrokSettingsReader.oauthTokenEnvironmentKey)
        } else {
            Issue.record("expected environment token injection")
        }
        #expect(
            support?.envOverride(token: "pasted-token") == [
                GrokSettingsReader.oauthTokenEnvironmentKey: "pasted-token",
            ])
        #expect(
            support?.envOverride(token: "Bearer abc.def.ghi") == [
                GrokSettingsReader.oauthTokenEnvironmentKey: "abc.def.ghi",
            ])
        #expect(support?.envOverride(token: "Cookie: sso=abc") == nil)
    }

    @Test
    func `classifies bearer, cookie, and management-key secrets`() {
        #expect(
            GrokCredentialRouting.resolve(
                tokenAccountToken: "Bearer abc.def", manualCookieHeader: nil)
                == .oauth(accessToken: "abc.def"))
        #expect(
            GrokCredentialRouting.resolve(
                tokenAccountToken: "Cookie: sso=abc", manualCookieHeader: nil)
                == .webCookie(header: "sso=abc"))
        #expect(
            GrokCredentialRouting.resolve(
                tokenAccountToken: "xai-mgmt-key", manualCookieHeader: nil) == .none)
        #expect(
            GrokCredentialRouting.resolve(
                tokenAccountToken: nil, manualCookieHeader: "sso=abc; sso-rw=def")
                == .webCookie(header: "sso=abc; sso-rw=def"))
    }

    @Test
    func `selected SuperGrok accounts remap to oauth or web, never an empty oauth pipeline`() {
        let adapter = GrokProviderDescriptor.descriptor.credentials
        let oauthAccount = ProviderTokenAccount(
            id: UUID(),
            label: "oauth",
            token: "pasted-token",
            addedAt: 0,
            lastUsed: nil)
        let cookieAccount = ProviderTokenAccount(
            id: UUID(),
            label: "cookie",
            token: "Cookie: sso=abc",
            addedAt: 0,
            lastUsed: nil)
        #expect(
            adapter?.selectedAccountSourceMode(base: .auto, account: oauthAccount, config: nil)
                == .oauth)
        #expect(
            adapter?.selectedAccountSourceMode(base: .auto, account: cookieAccount, config: nil)
                == .web)
        #expect(adapter?.selectedAccountSourceMode(base: .auto, account: nil, config: nil) == .auto)
        #expect(
            GrokProviderDescriptor.descriptor.fetchPlan.sourceModes == [.auto, .cli, .oauth, .web])
    }
}
