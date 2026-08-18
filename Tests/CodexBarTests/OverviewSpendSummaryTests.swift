import CodexBarCore
import Foundation
import Testing
@testable import CodexBar

struct OverviewSpendSummaryTests {
    @Test
    func `summary marks incomplete provider coverage as partial`() {
        let group = self.group(
            providers: [
                self.provider(.codex, tokens: 4_800_000, cost: 412.64),
                self.provider(.claude, tokens: nil, cost: nil),
                self.provider(.openrouter, tokens: 9_640_000, cost: 282.74),
                self.provider(.cursor, tokens: 1_250_000, cost: 64.18),
            ],
            totalTokens: nil,
            totalCost: nil)

        let summary = OverviewSpendSummary(
            model: SpendDashboardModel(requestedDays: 30, groups: [group]),
            providerCount: 4)

        #expect(summary.primarySpendText == "~$759.56")
        #expect(summary.coverageText == "3 / 4 Providers")
        #expect(summary.tokenText == "~15.7M tokens")
        #expect(summary.isPartial)
    }

    @Test
    func `summary keeps distinct currencies separate`() {
        let usd = self.group(
            currencyCode: "USD",
            providers: [self.provider(.codex, tokens: 1000, cost: 12)],
            totalTokens: 1000,
            totalCost: 12)
        let eur = self.group(
            currencyCode: "EUR",
            providers: [self.provider(.claude, tokens: 2000, cost: 8)],
            totalTokens: 2000,
            totalCost: 8)

        let summary = OverviewSpendSummary(
            model: SpendDashboardModel(requestedDays: 7, groups: [eur, usd]),
            providerCount: 2)

        #expect(summary.primarySpendText.contains("$12.00"))
        #expect(summary.primarySpendText.contains("€8.00"))
        #expect(summary.coverageText == "2 / 2 Providers")
        #expect(summary.tokenText == "3K tokens")
        #expect(!summary.isPartial)
    }

    private func provider(
        _ provider: UsageProvider,
        tokens: Int?,
        cost: Double?) -> SpendDashboardModel.ProviderRow
    {
        SpendDashboardModel.ProviderRow(
            id: provider.rawValue,
            rank: 1,
            provider: provider,
            displayName: provider.rawValue,
            totalTokens: tokens,
            totalCost: cost,
            coveredDayCount: 30)
    }

    private func group(
        currencyCode: String = "USD",
        providers: [SpendDashboardModel.ProviderRow],
        totalTokens: Int?,
        totalCost: Double?) -> SpendDashboardModel.CurrencyGroup
    {
        SpendDashboardModel.CurrencyGroup(
            currencyCode: currencyCode,
            providers: providers,
            models: [],
            projects: [],
            dailyPoints: [],
            totalTokens: totalTokens,
            totalCost: totalCost,
            coveredDayCount: 30,
            chartDomain: Date(timeIntervalSince1970: 0)...Date(timeIntervalSince1970: 86400),
            modelHistoryCompleteness: totalCost == nil ? .incomplete : .complete)
    }
}
