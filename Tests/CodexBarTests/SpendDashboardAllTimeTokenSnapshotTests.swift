import Foundation
import Testing
@testable import CodexBar
@testable import CodexBarCore

@MainActor
struct SpendDashboardAllTimeTokenSnapshotTests {
    @Test
    func `Claude spend dashboard scans all-time without publishing the menu window`() async throws {
        try await self.expectIndependentAllTimeScan(provider: .claude)
    }

    @Test
    func `Cursor spend dashboard scans all-time without publishing the menu window`() async throws {
        try await self.expectIndependentAllTimeScan(provider: .cursor)
    }

    private func expectIndependentAllTimeScan(provider: UsageProvider) async throws {
        let (settings, store) = Self.store(provider: provider)
        settings.costUsageHistoryDays = 30
        let calendar = Self.gmtCalendar
        let now = try #require(calendar.date(from: DateComponents(year: 2026, month: 8, day: 17, hour: 12)))
        let recentDay = try #require(Self.dayString(now, calendar: calendar))
        let oldDate = try #require(calendar.date(byAdding: .day, value: -40, to: now))
        let oldDay = try #require(Self.dayString(oldDate, calendar: calendar))
        var receivedHistoryDays: [Int] = []
        store._setTokenSnapshotForTesting(
            Self.snapshot(days: [recentDay], historyDays: 30, now: now),
            provider: provider)
        store._test_tokenUsageSnapshotLoaderOverride = { _, _, _, _, historyDays in
            receivedHistoryDays.append(historyDays)
            return Self.snapshot(days: [oldDay, recentDay], historyDays: historyDays, now: now)
        }

        let request = await SpendDashboardSource.makeRequest(
            settings: settings,
            store: store,
            mode: .forceRefresh,
            now: now)

        #expect(receivedHistoryDays == [SpendDashboardSource.scanDays])
        #expect(store.tokenSnapshotForCurrentProviderConfig(for: provider)?.snapshot.historyDays == 30)
        let captured = try #require(request.capturedInputs.first)
        #expect(captured.snapshot.historyDays == SpendDashboardSource.scanDays)
        #expect(captured.snapshot.daily.map(\.date) == [oldDay, recentDay])

        let thirty = SpendDashboardModel.build(
            inputs: request.capturedInputs,
            requestedDays: 30,
            now: now,
            calendar: calendar)
        let allTime = SpendDashboardModel.build(
            inputs: request.capturedInputs,
            requestedDays: SpendDashboardSource.scanDays,
            now: now,
            calendar: calendar)
        #expect(thirty.groups.first?.dailyPoints.contains { calendar.isDate($0.day, inSameDayAs: oldDate) } != true)
        #expect(allTime.groups.first?.dailyPoints.contains { calendar.isDate($0.day, inSameDayAs: oldDate) } == true)
        #expect(thirty.groups.first?.totalCost == 1)
        #expect(allTime.groups.first?.totalCost == 2)
    }

    private static func store(provider: UsageProvider) -> (SettingsStore, UsageStore) {
        let settings = testSettingsStore(suiteName: "SpendDashboardAllTimeTokenSnapshotTests-\(provider.rawValue)")
        settings.costUsageEnabled = true
        for candidate in UsageProvider.allCases {
            guard let metadata = ProviderRegistry.shared.metadata[candidate] else { continue }
            settings.setProviderEnabled(provider: candidate, metadata: metadata, enabled: candidate == provider)
        }
        let store = UsageStore(
            fetcher: UsageFetcher(environment: [:]),
            browserDetection: BrowserDetection(cacheTTL: 0),
            settings: settings,
            startupBehavior: .testing,
            environmentBase: [:])
        return (settings, store)
    }

    private static func snapshot(
        days: [String],
        historyDays: Int,
        now: Date) -> CostUsageTokenSnapshot
    {
        let entries = days.map { day in
            CostUsageDailyReport.Entry(
                date: day,
                inputTokens: nil,
                outputTokens: nil,
                totalTokens: 10,
                costUSD: 1,
                modelsUsed: nil,
                modelBreakdowns: nil)
        }
        return CostUsageTokenSnapshot(
            sessionTokens: nil,
            sessionCostUSD: nil,
            last30DaysTokens: 10 * days.count,
            last30DaysCostUSD: Double(days.count),
            currencyCode: "USD",
            historyDays: historyDays,
            daily: entries,
            updatedAt: now)
    }

    private static func dayString(_ date: Date, calendar: Calendar) -> String? {
        let parts = calendar.dateComponents([.year, .month, .day], from: date)
        guard let year = parts.year, let month = parts.month, let day = parts.day else { return nil }
        return String(format: "%04d-%02d-%02d", year, month, day)
    }

    private static var gmtCalendar: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(secondsFromGMT: 0) ?? .gmt
        return calendar
    }
}
