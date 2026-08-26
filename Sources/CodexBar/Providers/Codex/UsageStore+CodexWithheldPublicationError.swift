import CodexBarCore
import Foundation

extension UsageStore {
    /// Withholding a Codex snapshot is a decision about that reading, not about connectivity: the
    /// fetch behind it succeeded. The previously recorded failure must therefore not survive it.
    ///
    /// Without this, a transient outage becomes permanent on screen. The weekly-reset guard can keep
    /// withholding every later reading for as long as the account stays at or below the reset
    /// threshold, and each withheld cycle returns before the publication path that clears `errors`.
    /// The message is also persisted next to the preserved account snapshot and rehydrated at launch,
    /// so relaunching cannot clear it either — the card keeps reporting that the network is offline
    /// while it is in fact being refreshed successfully every cycle.
    func clearCodexFetchErrorAfterWithheldPublication(
        outcome: ProviderFetchOutcome,
        expectedGuard: CodexAccountScopedRefreshGuard?)
    {
        guard case .success = outcome.result else { return }
        // The streak counts fetch outcomes, not publications, and the ordinary publication path
        // records this success. Leaving it standing would spend the next transient failure's
        // first-failure suppression on an outage that already ended.
        self.failureGates[.codex]?.recordSuccess()
        if let recorded = self.errors[.codex], Self.codexErrorDisprovedBySuccessfulFetch(recorded) {
            self.errors[.codex] = nil
        }
        guard let accountID = self.codexAccountIDForWithheldPublication(expectedGuard: expectedGuard) else {
            return
        }
        if let index = self.codexAccountSnapshots.firstIndex(where: { $0.id == accountID }),
           let recorded = self.codexAccountSnapshots[index].error,
           Self.codexErrorDisprovedBySuccessfulFetch(recorded)
        {
            self.codexAccountSnapshots[index] = Self.clearingRecordedError(self.codexAccountSnapshots[index])
        }

        // A single-account Codex refresh empties the in-memory records for its duration, so the
        // persisted copy is amended from the store's own contents. Writing the in-memory array here
        // would erase the preserved snapshot this withheld cycle is deliberately keeping.
        guard let snapshotStore = self.codexAccountUsageSnapshotStore else { return }
        var persisted = snapshotStore.load(for: self.freshCodexVisibleAccountsForSnapshotHydration())
        guard let index = persisted.firstIndex(where: { $0.id == accountID }),
              let recorded = persisted[index].error,
              Self.codexErrorDisprovedBySuccessfulFetch(recorded)
        else { return }
        persisted[index] = Self.clearingRecordedError(persisted[index])
        snapshotStore.store(persisted)
    }

    /// Resolve the stored record this refresh speaks for, using the same account scoping the
    /// weekly-reset candidate persistence applies, so one account's success never clears another's
    /// recorded failure.
    private func codexAccountIDForWithheldPublication(
        expectedGuard: CodexAccountScopedRefreshGuard?) -> String?
    {
        guard let expectedGuard else { return nil }
        let currentGuard = self.freshCodexAccountScopedRefreshGuard()
        guard Self.codexScopedRefreshGuardsMatchAccount(expectedGuard, currentGuard) else { return nil }

        let activeMatches = self.freshCodexVisibleAccountsForSnapshotHydration().filter {
            $0.isActive &&
                $0.selectionSource == currentGuard.source &&
                CodexIdentityResolver.normalizeEmail($0.email) == currentGuard.accountKey
        }
        guard activeMatches.count == 1, let account = activeMatches.first else { return nil }
        return account.id
    }

    /// Only a connectivity claim is cleared here. A successful fetch is direct evidence against
    /// exactly that message — it is the same classification that allowed the message to be stored
    /// beside preserved usage in the first place. Every other kind (auth, workspace, parse) is left
    /// alone, so a withheld publication stays otherwise inert with respect to published state.
    private static func codexErrorDisprovedBySuccessfulFetch(_ message: String) -> Bool {
        shouldPreserveCodexAccountSnapshotOnFailure(message)
    }

    private static func clearingRecordedError(
        _ record: CodexAccountUsageSnapshot) -> CodexAccountUsageSnapshot
    {
        CodexAccountUsageSnapshot(
            account: record.account,
            snapshot: record.snapshot,
            error: nil,
            sourceLabel: record.sourceLabel,
            credits: record.credits,
            weeklyResetCandidate: record.weeklyResetCandidate)
    }
}
