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
}
