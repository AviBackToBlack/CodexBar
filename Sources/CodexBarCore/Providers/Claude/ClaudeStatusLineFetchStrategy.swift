import Foundation

/// Supplies an anonymous ambient Claude card only while the user's global Keychain-disable preference is on.
/// The provider descriptor excludes explicit credentials, non-Auto sources, and multi-account presentations
/// before this strategy can enter the chain.
struct ClaudeStatusLineFetchStrategy: ProviderFetchStrategy {
    typealias ObservationLoader = @Sendable (ProviderFetchContext) -> ClaudeStatusLineRateLimits?

    #if DEBUG
    @TaskLocal static var observationLoaderOverrideForTesting: ObservationLoader?
    #endif

    let id = "claude.statusline.standalone"
    let kind: ProviderFetchKind = .localProbe

    func isAvailable(_ context: ProviderFetchContext) async -> Bool {
        self.loadObservation(context) != nil
    }

    func fetch(_ context: ProviderFetchContext) async throws -> ProviderFetchResult {
        guard let limits = self.loadObservation(context),
              let snapshot = ClaudeStatusLineDropStore.makeSnapshot(from: limits)
        else { throw ClaudeStatusLineFetchError.noFreshObservation }
        return self.makeResult(
            usage: snapshot,
            sourceLabel: ClaudeStatusLineFeed.standaloneSourceLabel)
    }

    func shouldFallback(on error: Error, context _: ProviderFetchContext) -> Bool {
        !ClaudeOAuthFetchError.isCancellation(error)
    }

    private func loadObservation(_ context: ProviderFetchContext) -> ClaudeStatusLineRateLimits? {
        #if DEBUG
        if let override = Self.observationLoaderOverrideForTesting {
            return override(context)
        }
        #endif
        guard let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
        else { return nil }
        return ClaudeStatusLineDropStore.load(
            applicationSupport: support,
            expectedProfileID: ClaudeStatusLineProfile.identifier(environment: context.env))
    }
}

enum ClaudeStatusLineFetchError: Error {
    case noFreshObservation
}
