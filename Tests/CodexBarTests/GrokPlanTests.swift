import Foundation
import Testing
@testable import CodexBarCore

struct GrokPlanTests {
    @Test
    func `normalizes SuperGrok Heavy subscription tiers`() {
        #expect(GrokPlan.displayName(from: "SuperGrok Heavy") == "SuperGrok Heavy")
        #expect(GrokPlan.displayName(from: "supergrok_heavy") == "SuperGrok Heavy")
        #expect(GrokPlan.displayName(from: "  HEAVY  ") == "SuperGrok Heavy")
        #expect(GrokPlan.displayName(from: "SuperGrok") == "SuperGrok")
        #expect(GrokPlan.displayName(from: "Custom Team") == "Custom Team")
        #expect(GrokPlan.displayName(from: "   ") == nil)
        #expect(GrokPlan.displayName(from: nil) == nil)
    }

    @Test
    func `treats Heavy omitted credit percent as unknown usage`() {
        #expect(GrokPlan.omitsIncludedUsagePercent("SuperGrok Heavy"))
        #expect(GrokPlan.omitsIncludedUsagePercent("supergrok_heavy"))
        #expect(!GrokPlan.omitsIncludedUsagePercent("SuperGrok"))
        #expect(!GrokPlan.omitsIncludedUsagePercent(nil))
    }

    @Test
    func `login method prefers billing tier over OIDC SuperGrok`() {
        let credentials = GrokCredentials(
            accessToken: "token",
            refreshToken: nil,
            scope: "https://auth.x.ai::client",
            authMode: "oidc",
            userId: nil,
            email: "grok@example.com",
            firstName: nil,
            lastName: nil,
            teamId: nil,
            oidcIssuer: nil,
            oidcClientId: nil,
            expiresAt: nil,
            createTime: nil)

        #expect(credentials.loginMethod == "SuperGrok")
        #expect(GrokPlan.loginMethod(subscriptionTier: "SuperGrok Heavy", credentials: credentials)
            == "SuperGrok Heavy")
        #expect(GrokPlan.loginMethod(subscriptionTier: nil, credentials: credentials) == "SuperGrok")
        #expect(GrokPlan.loginMethod(subscriptionTier: "  ", credentials: credentials) == "SuperGrok")
    }

    @Test
    func `settings parser reads subscription_tier_display`() {
        #expect(GrokCLISettingsFetcher.parse(Data(#"{"subscription_tier_display":"SuperGrok Heavy"}"#.utf8))
            == "SuperGrok Heavy")
        #expect(GrokCLISettingsFetcher.parse(Data(#"{"subscription_tier_display":"supergrok"}"#.utf8))
            == "SuperGrok")
        #expect(GrokCLISettingsFetcher.parse(Data(#"{}"#.utf8)) == nil)
        #expect(GrokCLISettingsFetcher.parse(Data("not-json".utf8)) == nil)
    }

    @Test
    func `applying Heavy drops an inferred zero percent`() {
        let inferred = GrokWebBillingSnapshot(
            usedPercent: 0,
            resetsAt: Date(timeIntervalSince1970: 1_800_000_000),
            subscriptionTier: nil,
            inferredZeroUsage: true)
        let applied = inferred.applying(subscriptionTier: "SuperGrok Heavy")
        #expect(applied.subscriptionTier == "SuperGrok Heavy")
        #expect(applied.usedPercent == nil)
        #expect(applied.resetsAt == Date(timeIntervalSince1970: 1_800_000_000))

        let explicit = GrokWebBillingSnapshot(
            usedPercent: 0,
            resetsAt: Date(timeIntervalSince1970: 1_800_000_000),
            subscriptionTier: nil)
        #expect(explicit.applying(subscriptionTier: "SuperGrok Heavy").usedPercent == 0)
    }

    @Test
    func `settings endpoint is derived from the billing host`() throws {
        let billing = try #require(URL(string: "https://grok.test/v1/billing?format=credits"))
        #expect(GrokCLISettingsFetcher.endpoint(fromBilling: billing).absoluteString
            == "https://grok.test/v1/settings")
    }
}
