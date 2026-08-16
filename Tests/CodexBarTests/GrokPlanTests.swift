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
}
