import CodexBarCore
import Foundation
import Testing
@testable import CodexBar

@MainActor
extension CodexAccountScopedRefreshTests {
    @Test
    func `withheld weekly reset clears the failure recorded before it`() async throws {
        let suite = "CodexWithheldPublicationErrorTests-clears-stale-failure"
        let email = "withheld-stale-failure@example.com"
        let settings = self.makeSettingsStore(suite: suite)
        settings.refreshFrequency = .manual
        settings.codexCookieSource = .off
        settings._test_liveSystemCodexAccount = self.liveAccount(
            email: email,
            identity: .providerAccount(id: "acct-withheld-stale-failure"))
        defer { settings._test_liveSystemCodexAccount = nil }

        let now = Date()
        let priorBoundary = now.addingTimeInterval(2 * 24 * 60 * 60)
        let nextBoundary = priorBoundary.addingTimeInterval(7 * 24 * 60 * 60)
        let creditExpiry = nextBoundary.addingTimeInterval(24 * 60 * 60)
        let prior = self.codexWeeklySnapshot(
            email: email,
            weeklyUsedPercent: 81,
            weeklyReset: priorBoundary,
            updatedAt: now.addingTimeInterval(-600),
            resetCredits: withheldPublicationResetCredits(
                capturedAt: now.addingTimeInterval(-600),
                expiresAt: creditExpiry),
            dataConfidence: .exact)
        // The post-reset reading the weekly guard withholds: at or below the reset threshold with an
        // advanced boundary, while the reset credit stays available.
        let postReset = self.codexWeeklySnapshot(
            email: email,
            weeklyUsedPercent: 0,
            weeklyReset: nextBoundary,
            updatedAt: now.addingTimeInterval(-60),
            resetCredits: withheldPublicationResetCredits(
                capturedAt: now.addingTimeInterval(-60),
                expiresAt: creditExpiry),
            dataConfidence: .exact)

        let snapshotURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("codex-withheld-error-\(UUID().uuidString).json")
        defer { try? FileManager.default.removeItem(at: snapshotURL) }
        let snapshotStore = FileCodexAccountUsageSnapshotStore(fileURL: snapshotURL)

        let seedStore = self.makeCodexWeeklyPublicationStore(
            settings: settings,
            suite: suite,
            snapshotStore: snapshotStore)
        self.installContextualCodexProvider(on: seedStore, sourceLabel: "oauth", kind: .oauth) { _ in prior }
        await seedStore.refreshProvider(.codex, allowDisabled: true)
        #expect(seedStore.errors[.codex] == nil)

        // An outage records a failure next to the preserved snapshot — the shape
        // `resolvedCodexAccountOutcome` persists for a network failure that keeps prior usage.
        let visibleAccounts = settings.codexVisibleAccountProjection.visibleAccounts
        let seeded = try #require(snapshotStore.load(for: visibleAccounts).first)
        snapshotStore.store([CodexAccountUsageSnapshot(
            account: seeded.account,
            snapshot: seeded.snapshot,
            error: "Network error: The Internet connection appears to be offline.",
            sourceLabel: seeded.sourceLabel,
            credits: seeded.credits)])
        let persistedFailure = try #require(snapshotStore.load(for: visibleAccounts).first)
        #expect(persistedFailure.error != nil)
        #expect(persistedFailure.snapshot?.updatedAt == prior.updatedAt)

        // Relaunch: the recorded failure is rehydrated, then the next fetch succeeds but its reading
        // is withheld by the weekly-reset guard.
        let relaunchedStore = self.makeCodexWeeklyPublicationStore(
            settings: settings,
            suite: suite,
            snapshotStore: snapshotStore)
        self.installContextualCodexProvider(on: relaunchedStore, sourceLabel: "oauth", kind: .oauth) { _ in
            postReset
        }
        await CodexWeeklyResetConfirmation.$observationDateOverride.withValue(postReset.updatedAt) {
            await relaunchedStore.refreshProvider(.codex, allowDisabled: true)
        }

        // The withheld reading still does not replace the published snapshot.
        #expect(relaunchedStore.snapshots[.codex]?.updatedAt == prior.updatedAt)
        // ...but the failure it superseded is gone, in memory and on disk, so the card cannot keep
        // reporting an outage while it is being refreshed successfully.
        #expect(relaunchedStore.errors[.codex] == nil)
        let persistedAfterWithhold = try #require(snapshotStore.load(
            for: settings.codexVisibleAccountProjection.visibleAccounts).first)
        #expect(persistedAfterWithhold.error == nil)
        // The preserved snapshot must survive on disk: a withheld cycle amends the recorded failure,
        // it does not rewrite the store from the in-memory records a single-account refresh clears.
        #expect(persistedAfterWithhold.snapshot?.updatedAt == prior.updatedAt)
    }

    @Test
    func `withheld publication keeps a failure recorded by the same refresh`() async {
        let suite = "CodexWithheldPublicationErrorTests-keeps-current-failure"
        let email = "withheld-current-failure@example.com"
        let settings = self.makeSettingsStore(suite: suite)
        settings.refreshFrequency = .manual
        settings.codexCookieSource = .off
        settings._test_liveSystemCodexAccount = self.liveAccount(
            email: email,
            identity: .providerAccount(id: "acct-withheld-current-failure"))
        defer { settings._test_liveSystemCodexAccount = nil }

        let now = Date()
        let prior = self.codexWeeklySnapshot(
            email: email,
            weeklyUsedPercent: 81,
            weeklyReset: now.addingTimeInterval(2 * 24 * 60 * 60),
            updatedAt: now.addingTimeInterval(-600),
            dataConfidence: .exact)

        let snapshotURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("codex-withheld-error-current-\(UUID().uuidString).json")
        defer { try? FileManager.default.removeItem(at: snapshotURL) }
        let snapshotStore = FileCodexAccountUsageSnapshotStore(fileURL: snapshotURL)

        let seedStore = self.makeCodexWeeklyPublicationStore(
            settings: settings,
            suite: suite,
            snapshotStore: snapshotStore)
        self.installContextualCodexProvider(on: seedStore, sourceLabel: "oauth", kind: .oauth) { _ in prior }
        await seedStore.refreshProvider(.codex, allowDisabled: true)

        let failingStore = self.makeCodexWeeklyPublicationStore(
            settings: settings,
            suite: suite,
            snapshotStore: snapshotStore)
        self.installContextualCodexProvider(on: failingStore, sourceLabel: "oauth", kind: .oauth) { _ in
            throw withheldPublicationNetworkError()
        }
        await failingStore.refreshProvider(.codex, allowDisabled: true)
        await failingStore.refreshProvider(.codex, allowDisabled: true)

        // A failing fetch is not a withheld success: its message must survive to explain the stale card.
        #expect(failingStore.errors[.codex] != nil)
        #expect(failingStore.snapshots[.codex]?.updatedAt == prior.updatedAt)
    }

    @Test
    func `a withheld success restores first-failure suppression`() async {
        let suite = "CodexWithheldPublicationErrorTests-gate-recovery"
        let email = "withheld-gate-recovery@example.com"
        let settings = self.makeSettingsStore(suite: suite)
        settings.refreshFrequency = .manual
        settings.codexCookieSource = .off
        settings._test_liveSystemCodexAccount = self.liveAccount(
            email: email,
            identity: .providerAccount(id: "acct-withheld-gate-recovery"))
        defer { settings._test_liveSystemCodexAccount = nil }

        let now = Date()
        let priorBoundary = now.addingTimeInterval(2 * 24 * 60 * 60)
        let nextBoundary = priorBoundary.addingTimeInterval(7 * 24 * 60 * 60)
        let creditExpiry = nextBoundary.addingTimeInterval(24 * 60 * 60)
        let prior = self.codexWeeklySnapshot(
            email: email,
            weeklyUsedPercent: 81,
            weeklyReset: priorBoundary,
            updatedAt: now.addingTimeInterval(-600),
            resetCredits: withheldPublicationResetCredits(
                capturedAt: now.addingTimeInterval(-600),
                expiresAt: creditExpiry),
            dataConfidence: .exact)
        let postReset = self.codexWeeklySnapshot(
            email: email,
            weeklyUsedPercent: 0,
            weeklyReset: nextBoundary,
            updatedAt: now.addingTimeInterval(-60),
            resetCredits: withheldPublicationResetCredits(
                capturedAt: now.addingTimeInterval(-60),
                expiresAt: creditExpiry),
            dataConfidence: .exact)

        let snapshotURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("codex-withheld-gate-\(UUID().uuidString).json")
        defer { try? FileManager.default.removeItem(at: snapshotURL) }
        let snapshotStore = FileCodexAccountUsageSnapshotStore(fileURL: snapshotURL)

        let seedStore = self.makeCodexWeeklyPublicationStore(
            settings: settings,
            suite: suite,
            snapshotStore: snapshotStore)
        self.installContextualCodexProvider(on: seedStore, sourceLabel: "oauth", kind: .oauth) { _ in prior }
        await seedStore.refreshProvider(.codex, allowDisabled: true)

        let script = WithheldPublicationFetchScript(success: postReset)
        let store = self.makeCodexWeeklyPublicationStore(
            settings: settings,
            suite: suite,
            snapshotStore: snapshotStore)
        self.installContextualCodexProvider(on: store, sourceLabel: "oauth", kind: .oauth) { _ in
            try await script.load()
        }

        // Two consecutive failures: the first is suppressed, the second surfaces.
        await store.refreshProvider(.codex, allowDisabled: true)
        await store.refreshProvider(.codex, allowDisabled: true)
        #expect(store.errors[.codex] != nil)

        // Connectivity returns, but the reading is withheld by the weekly-reset guard.
        await script.setFailing(false)
        await CodexWeeklyResetConfirmation.$observationDateOverride.withValue(postReset.updatedAt) {
            await store.refreshProvider(.codex, allowDisabled: true)
        }
        #expect(store.errors[.codex] == nil)
        #expect(store.snapshots[.codex]?.updatedAt == prior.updatedAt)

        // One later transient failure must get first-failure suppression again, exactly as it would
        // after an ordinary published success.
        await script.setFailing(true)
        await store.refreshProvider(.codex, allowDisabled: true)
        #expect(store.errors[.codex] == nil)
        #expect(store.snapshots[.codex]?.updatedAt == prior.updatedAt)
    }

    @Test
    func `a withheld publication clears only a connectivity claim`() {
        // A successful fetch is evidence against an offline claim and nothing else, so only that
        // class is eligible. Everything else keeps a withheld publication inert, which is what
        // `matching weekly lows before the prior reset remain private` pins down.
        #expect(UsageStore.shouldPreserveCodexAccountSnapshotOnFailure(
            "Network error: The Internet connection appears to be offline."))
        #expect(UsageStore.shouldPreserveCodexAccountSnapshotOnFailure("Request timed out"))
        #expect(!UsageStore.shouldPreserveCodexAccountSnapshotOnFailure("prior error"))
        #expect(!UsageStore.shouldPreserveCodexAccountSnapshotOnFailure("401 unauthorized"))
        #expect(!UsageStore.shouldPreserveCodexAccountSnapshotOnFailure("Workspace deactivated"))
    }
}

/// Mirrors production on both axes that matter here. The transport domain and code are what
/// `isPreservableNetworkTransportError` keeps the prior snapshot for — and prior data is what earns a
/// failure its first-failure suppression. The message is the shape the Codex OAuth fetcher produces,
/// `"Network error: <localizedDescription>"`, which is what makes it connectivity-classified; a bare
/// `URLError` renders as `(NSURLErrorDomain error -1009.)` in a test process and would not be.
private func withheldPublicationNetworkError() -> Error {
    NSError(
        domain: NSURLErrorDomain,
        code: NSURLErrorNotConnectedToInternet,
        userInfo: [
            NSLocalizedDescriptionKey: "Network error: The Internet connection appears to be offline.",
        ])
}

private actor WithheldPublicationFetchScript {
    private var failing = true
    private let success: UsageSnapshot

    init(success: UsageSnapshot) {
        self.success = success
    }

    func setFailing(_ value: Bool) {
        self.failing = value
    }

    func load() throws -> UsageSnapshot {
        if self.failing {
            throw withheldPublicationNetworkError()
        }
        return self.success
    }
}

private func withheldPublicationResetCredits(
    capturedAt: Date,
    expiresAt: Date) -> CodexRateLimitResetCreditsSnapshot
{
    CodexRateLimitResetCreditsSnapshot(
        credits: [CodexRateLimitResetCredit(
            id: "withheld-publication-reset-credit",
            resetType: "codex_rate_limits",
            status: .available,
            grantedAt: capturedAt.addingTimeInterval(-24 * 60 * 60),
            expiresAt: expiresAt,
            redeemStartedAt: nil,
            redeemedAt: nil,
            title: nil,
            description: nil)],
        availableCount: 1,
        updatedAt: capturedAt)
}
