import CodexBarCore
import Foundation
import Testing
@testable import CodexBar

struct LiteLLMKeyBudgetTests {
    @Test
    func `parses virtual key budget fields from key info`() throws {
        let json = """
        {
          "key": "sk-redacted",
          "info": {
            "key_name": "sk-...redacted",
            "spend": 2.26189835,
            "expires": null,
            "user_id": null,
            "team_id": "team-redacted",
            "max_budget": 200.0,
            "budget_duration": "1mo",
            "budget_reset_at": "2026-10-01T00:00:00+00:00"
          }
        }
        """

        let parsed = try LiteLLMUsageFetcher._parseKeyInfoForTesting(Data(json.utf8))

        #expect(parsed.userID == nil)
        #expect(parsed.teamID == "team-redacted")
        #expect(parsed.spendUSD == 2.26189835)
        #expect(parsed.budgetUSD == 200)
        #expect(parsed.budgetDuration == "1mo")
        #expect(parsed.budgetResetAt != nil)
    }

    @Test
    func `team-only virtual key budget becomes primary usage window`() throws {
        let teamJSON = """
        {
          "team_id": "team-redacted",
          "team_info": {
            "team_id": "team-redacted",
            "team_alias": "platform",
            "max_budget": null,
            "spend": 123.0,
            "budget_duration": null,
            "budget_reset_at": null
          }
        }
        """
        let keyInfo = LiteLLMKeyInfoSnapshot(
            userID: nil,
            teamID: "team-redacted",
            keyName: "sk-...redacted",
            spendUSD: 2.26189835,
            expiresAt: nil,
            budgetUSD: 200,
            budgetResetAt: ISO8601DateFormatter().date(from: "2026-10-01T00:00:00Z"),
            budgetDuration: "1mo")

        let parsed = try LiteLLMUsageFetcher._parseTeamInfoForTesting(
            Data(teamJSON.utf8),
            keyInfo: keyInfo,
            updatedAt: Date(timeIntervalSince1970: 1))
        let usage = parsed.toUsageSnapshot()

        let primary = try #require(usage.primary)
        #expect(abs(primary.usedPercent - 1.130949175) < 0.000001)
        #expect(primary.resetDescription == "$2.26 / $200.00")
        #expect(primary.resetsAt == keyInfo.budgetResetAt)
        #expect(usage.secondary == nil)
        #expect(usage.tertiary == nil)
        #expect(usage.providerCost?.used == 2.26189835)
        #expect(usage.providerCost?.limit == 200)
        #expect(usage.providerCost?.period == "Key budget")
    }

    @Test
    func `team-only virtual key budget is labeled as key budget in menu card`() throws {
        let now = Date(timeIntervalSince1970: 0)
        let reset = now.addingTimeInterval(30 * 24 * 60 * 60)
        let snapshot = UsageSnapshot(
            primary: RateWindow(
                usedPercent: 1.130949175,
                windowMinutes: nil,
                resetsAt: reset,
                resetDescription: "$2.26 / $200.00"),
            secondary: nil,
            tertiary: nil,
            providerCost: ProviderCostSnapshot(
                used: 2.26189835,
                limit: 200,
                currencyCode: "USD",
                period: "Key budget",
                resetsAt: reset,
                updatedAt: now),
            litellmBudget: LiteLLMBudgetContext(source: .key, primary: .key, secondary: nil, tertiary: nil),
            updatedAt: now)
        let metadata = try #require(ProviderDefaults.metadata[.litellm])

        let model = UsageMenuCardView.Model.make(.init(
            provider: .litellm,
            metadata: metadata,
            snapshot: snapshot,
            credits: nil,
            creditsError: nil,
            dashboard: nil,
            dashboardError: nil,
            tokenSnapshot: nil,
            tokenError: nil,
            account: AccountInfo(email: nil, plan: nil),
            isRefreshing: false,
            lastError: nil,
            usageBarsShowUsed: false,
            resetTimeDisplayStyle: .countdown,
            tokenCostUsageEnabled: false,
            showOptionalCreditsAndExtraUsage: true,
            hidePersonalInfo: false,
            now: now))

        let key = try #require(model.metrics.first { $0.id == "primary" })
        #expect(key.title == "Key budget")
        #expect(key.detailText == "$2.26 / $200.00")
        #expect(key.percentLabel == "99% left")
        #expect(key.resetText?.hasPrefix("Resets") == true)
        #expect(model.providerCost == nil)
    }

    @Test
    func `fetch preserves key budget when team has no budget`() async throws {
        let baseURL = try #require(URL(string: "https://litellm.example.com/v1"))
        let transport = ProviderHTTPTransportStub { request in
            let path = request.url?.path
            let body: String
            switch path {
            case "/key/info":
                body = """
                {
                  "info": {
                    "key_name": "sk-...redacted",
                    "user_id": null,
                    "team_id": "team-redacted",
                    "spend": 2.26189835,
                    "max_budget": 200,
                    "budget_duration": "1mo",
                    "budget_reset_at": "2026-10-01T00:00:00+00:00"
                  }
                }
                """
            case "/team/info":
                body = """
                {
                  "team_id": "team-redacted",
                  "team_info": {
                    "team_id": "team-redacted",
                    "team_alias": "platform",
                    "max_budget": null,
                    "spend": 123
                  }
                }
                """
            default:
                Issue.record("unexpected LiteLLM request path: \(path ?? "nil")")
                body = "{}"
            }

            let url = try #require(request.url)
            let response = try #require(HTTPURLResponse(
                url: url,
                statusCode: 200,
                httpVersion: nil,
                headerFields: nil))
            return (Data(body.utf8), response)
        }

        let parsed = try await LiteLLMUsageFetcher.fetchUsage(
            apiKey: "sk-test",
            baseURL: baseURL,
            transport: transport,
            updatedAt: Date(timeIntervalSince1970: 1))
        let usage = parsed.toUsageSnapshot()

        #expect(parsed.keyBudgetUSD == 200)
        #expect(parsed.keyBudgetDuration == "1mo")
        #expect(usage.primary?.resetDescription == "$2.26 / $200.00")
        #expect(usage.providerCost?.period == "Key budget")

        let requests = await transport.requests()
        #expect(requests.count == 2)
    }

    @Test
    @MainActor
    func `budget layouts preserve semantic labels and source through serialization`() throws {
        let metadata = try #require(ProviderDefaults.metadata[.litellm])
        for hasKey in [true, false] {
            for hasPersonal in [true, false] {
                for hasTeam in [true, false] {
                    let usage = try Self.snapshot(key: hasKey, personal: hasPersonal, team: hasTeam)
                    let encoded = try JSONEncoder().encode(usage)
                    let json = try #require(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
                    let budgetJSON = try #require(json["litellmBudget"] as? [String: Any])
                    #expect(budgetJSON["litellm.budget.source"] as? String == (hasKey ? "key" : "spend"))
                    let restored = try JSONDecoder().decode(UsageSnapshot.self, from: encoded)
                        .withSubscriptionMetadata(expiresAt: nil, renewsAt: nil)
                    #expect(restored.litellmBudget == usage.litellmBudget)
                    #expect(restored.primary?.usedPercent == (hasKey ? 10 : (hasPersonal ? 20 : nil)))
                    #expect(restored.secondary?.usedPercent ==
                        (hasKey && hasPersonal ? 20 : (hasTeam ? 30 : nil)))
                    #expect(restored.tertiary?.usedPercent == (hasKey && hasPersonal && hasTeam ? 30 : nil))
                    #expect(restored.providerCost?.used == (hasKey ? 10 : 20))
                    #expect(restored.providerCost?.limit == (hasKey || hasPersonal ? 100 : 0))

                    // This is the label-routing seam used by the native app menu.
                    let labels = MenuDescriptor.rateWindowLabels(
                        provider: .litellm, metadata: metadata, snapshot: restored)
                    #expect(labels.primary == (hasKey ? "Key budget" : "Personal budget"))
                    if restored.secondary != nil {
                        #expect(labels.secondary == (hasKey && hasPersonal ? "Personal budget" : "Team budget"))
                    }
                    if restored.tertiary != nil {
                        #expect(labels.tertiary == "Team budget")
                        #expect(labels.showsTertiary)
                    }
                }
            }
        }
    }

    @Test
    @MainActor
    func `budget labels do not depend on display descriptions`() throws {
        let usage = try Self.snapshot(key: true, personal: false, team: true)
        let renamed = UsageSnapshot(
            primary: usage.primary,
            secondary: RateWindow(usedPercent: 30, windowMinutes: nil, resetsAt: nil, resetDescription: "Localized"),
            providerCost: ProviderCostSnapshot(
                used: 10, limit: 100, currencyCode: "USD", period: "Localized", resetsAt: nil,
                updatedAt: usage.updatedAt),
            litellmBudget: usage.litellmBudget,
            updatedAt: usage.updatedAt)
        let labels = try MenuDescriptor.rateWindowLabels(
            provider: .litellm,
            metadata: #require(ProviderDefaults.metadata[.litellm]),
            snapshot: renamed)
        #expect(labels.primary == "Key budget")
        #expect(labels.secondary == "Team budget")
        #expect(renamed.litellmBudget?.source == .key)
    }

    private static func snapshot(key: Bool, personal: Bool, team: Bool) throws -> UsageSnapshot {
        let json = """
        {
          "user_info": {"user_id": "user-test", "spend": 20, "max_budget": \(personal ? "100" : "null")},
          "teams": [{"team_id": "team-test", "spend": 30, "max_budget": \(team ? "100" : "null")}]
        }
        """
        return try LiteLLMUsageFetcher._parseUserInfoForTesting(
            Data(json.utf8),
            keyInfo: LiteLLMKeyInfoSnapshot(
                userID: "user-test", teamID: "team-test", keyName: nil, spendUSD: 10, expiresAt: nil,
                budgetUSD: key ? 100 : nil),
            updatedAt: Date(timeIntervalSince1970: 1)).toUsageSnapshot()
    }
}
