import Foundation
import Testing
@testable import CodexBarCore

struct LiteLLMKeyBudgetPluginTests {
    private static let userWithTeam = #"{"user_info":{"user_id":"user-123","spend":4,"max_budget":100},"teams":[{"team_id":"team-456","team_alias":"TeamA","spend":40,"max_budget":1000}]}"#

    @Test(arguments: BundledPluginTestSupport.engines)
    func `virtual key budget is primary with independent personal and team windows`(
        engine: ProviderPluginEngineKind) async throws
    {
        let key = #"{"info":{"user_id":"user-123","team_id":"team-456","spend":12,"max_budget":20,"budget_reset_at":"2026-11-07T00:00:00Z"}}"#
        let snapshot = try await LiteLLMPluginTestSupport.fetch(Self.userWithTeam, key: key, engine: engine)
        #expect(snapshot.primary?.usedPercent == 60)
        #expect(snapshot.secondary?.usedPercent == 4)
        #expect(snapshot.tertiary?.usedPercent == 4)
        #expect(snapshot.providerCost?.used == 12)
        #expect(snapshot.providerCost?.limit == 20)
        #expect(snapshot.providerCost?.period == "Key budget")
        let roles = try #require(snapshot.litellmBudget)
        #expect(roles.source == .key)
        #expect(roles.primary == .key)
        #expect(roles.secondary == .personal)
        #expect(roles.tertiary == .team)
        let labels = LiteLLMProviderDescriptor.descriptor.presentation.rateWindowLabels(
            metadata: LiteLLMProviderDescriptor.descriptor.metadata, snapshot: snapshot)
        #expect(labels.primary == "Key budget")
        #expect(labels.secondary == "Personal budget")
        #expect(labels.tertiary == "Team budget")
        #expect(labels.showsTertiary)
        let decoded = try JSONDecoder().decode(UsageSnapshot.self, from: JSONEncoder().encode(snapshot))
        #expect(decoded.litellmBudget == roles)
    }

    @Test(arguments: BundledPluginTestSupport.engines)
    func `virtual key budget with team only labels its secondary window as team`(
        engine: ProviderPluginEngineKind) async throws
    {
        let key = #"{"info":{"team_id":"team-456","spend":3,"max_budget":10}}"#
        let team = #"{"team_info":{"team_id":"team-456","spend":1,"max_budget":100}}"#
        let snapshot = try await LiteLLMPluginTestSupport.fetch(team, key: key, engine: engine)
        #expect(snapshot.primary?.usedPercent == 30)
        #expect(snapshot.secondary?.usedPercent == 1)
        #expect(snapshot.tertiary == nil)
        #expect(snapshot.litellmBudget?.secondary == .team)
        let labels = LiteLLMProviderDescriptor.descriptor.presentation.rateWindowLabels(
            metadata: LiteLLMProviderDescriptor.descriptor.metadata, snapshot: snapshot)
        #expect(labels.primary == "Key budget")
        #expect(labels.secondary == "Team budget")
        #expect(!labels.showsTertiary)
    }

    @Test(arguments: BundledPluginTestSupport.engines)
    func `without positive key limit upstream personal and team behavior is unchanged`(
        engine: ProviderPluginEngineKind) async throws
    {
        let key = #"{"info":{"user_id":"user-123","team_id":"team-456","spend":12,"max_budget":null}}"#
        let snapshot = try await LiteLLMPluginTestSupport.fetch(Self.userWithTeam, key: key, engine: engine)
        #expect(snapshot.primary?.usedPercent == 4)
        #expect(snapshot.secondary?.usedPercent == 4)
        #expect(snapshot.tertiary == nil)
        #expect(snapshot.litellmBudget == nil)
        #expect(snapshot.providerCost?.used == 4)
        #expect(snapshot.providerCost?.limit == 100)
        let labels = LiteLLMProviderDescriptor.descriptor.presentation.rateWindowLabels(
            metadata: LiteLLMProviderDescriptor.descriptor.metadata, snapshot: snapshot)
        #expect(labels.primary == "Personal budget")
        #expect(labels.secondary == "Team budget")
    }
}
