import Foundation
import Testing
@testable import CodexBarCore

struct CostProvenanceTests {
    @Test
    func `cost figures are never billing receipts`() {
        for provenance in [CostProvenance.listPriceEstimate, .vendorMetered, .mixed, .unknown] {
            #expect(!provenance.isBillingReceipt)
        }
    }

    @Test
    func `coverage ratio ignores missing categories instead of collapsing them`() {
        let empty = CostUsageCoverageCounts()
        #expect(empty.coverageRatio == nil)

        let mixed = CostUsageCoverageCounts(priced: 2, unpriced: 1, unmetered: 1, estimated: 2)
        #expect(mixed.total == 6)
        #expect(mixed.coverageRatio == 4.0 / 6.0)
    }

    @Test
    func `token mix keeps nil distinct from zero`() {
        var mix = CostUsageTokenMix(inputTokens: 10, outputTokens: nil)
        #expect(mix.inputTokens == 10)
        #expect(mix.outputTokens == nil)
        mix.merge(CostUsageTokenMix(outputTokens: 4, reasoningTokens: 0))
        #expect(mix.outputTokens == 4)
        #expect(mix.reasoningTokens == 0)
        mix.merge(CostUsageTokenMix(inputTokens: 5))
        #expect(mix.inputTokens == 15)
        #expect(mix.cacheReadTokens == nil)
    }

    @Test
    func `day rows without request counts still expose priced or unpriced coverage`() {
        let priced = CostUsageDailyReport.Entry(
            date: "2026-07-16",
            inputTokens: 10,
            outputTokens: 2,
            totalTokens: 12,
            costUSD: 1.25,
            modelsUsed: nil,
            modelBreakdowns: nil)
        #expect(priced.coverageCounts == CostUsageCoverageCounts(priced: 1))

        let unpriced = CostUsageDailyReport.Entry(
            date: "2026-07-16",
            inputTokens: 10,
            outputTokens: 2,
            totalTokens: 12,
            costUSD: nil,
            modelsUsed: nil,
            modelBreakdowns: nil)
        #expect(unpriced.coverageCounts == CostUsageCoverageCounts(unpriced: 1))

        let unmetered = CostUsageDailyReport.Entry(
            date: "2026-07-16",
            inputTokens: nil,
            outputTokens: nil,
            totalTokens: nil,
            costUSD: nil,
            modelsUsed: nil,
            modelBreakdowns: nil,
            unmeteredRequestCount: 2)
        #expect(unmetered.coverageCounts == CostUsageCoverageCounts(unmetered: 2))
    }
}

struct CostUsageBucketTimeZoneTests {
    @Test
    func `pins a valid IANA identifier and rejects junk`() {
        #expect(CostUsageBucketTimeZone.isValidIdentifier("America/Los_Angeles"))
        #expect(!CostUsageBucketTimeZone.isValidIdentifier("Not/AZone"))
        let calendar = CostUsageBucketTimeZone.calendar(identifier: "America/Los_Angeles")
        #expect(calendar.timeZone.identifier == "America/Los_Angeles")
        #expect(calendar.identifier == .gregorian)
    }

    @Test
    func `a pinned zone keeps midnight-adjacent events on the same local day`() throws {
        let timestamp = "2026-07-16T06:30:00Z"
        var losAngeles = Calendar(identifier: .gregorian)
        losAngeles.timeZone = try #require(TimeZone(identifier: "America/Los_Angeles"))
        var shanghai = Calendar(identifier: .gregorian)
        shanghai.timeZone = try #require(TimeZone(identifier: "Asia/Shanghai"))
        let westKey = try #require(CostUsageScanner.dayKeyFromTimestamp(timestamp, calendar: losAngeles))
        let eastKey = try #require(CostUsageScanner.dayKeyFromTimestamp(timestamp, calendar: shanghai))
        #expect(westKey == "2026-07-15")
        #expect(eastKey == "2026-07-16")

        let pinned = CostUsageBucketTimeZone.calendar(identifier: "America/Los_Angeles")
        #expect(CostUsageScanner.dayKeyFromTimestamp(timestamp, calendar: pinned) == westKey)
        #expect(CostUsageScanner.dayKeyFromTimestamp(timestamp, calendar: pinned) != eastKey)
    }
}
