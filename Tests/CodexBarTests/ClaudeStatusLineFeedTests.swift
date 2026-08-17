import Foundation
import Testing
@testable import CodexBar
@testable import CodexBarCLI
@testable import CodexBarCore

@Suite(.serialized)
struct ClaudeStatusLineFeedTests {
    private let now = Date(timeIntervalSince1970: 1_800_000_000)

    private func limits(
        profileID: String = "profile-a",
        capturedAt: Date? = nil,
        fiveHour: Double? = 40,
        sevenDay: Double? = 60) -> ClaudeStatusLineRateLimits
    {
        ClaudeStatusLineRateLimits(
            profileID: profileID,
            capturedAt: capturedAt ?? self.now,
            fiveHour: fiveHour.map {
                ClaudeStatusLineWindow(usedPercent: $0, resetsAt: self.now.addingTimeInterval(3600))
            },
            sevenDay: sevenDay.map {
                ClaudeStatusLineWindow(usedPercent: $0, resetsAt: self.now.addingTimeInterval(86400))
            })
    }

    private func settings(
        enabled: Bool = true,
        keychainDisabled: Bool = true,
        standaloneAllowed: Bool = true,
        source: ClaudeUsageDataSource = .auto) -> ProviderSettingsSnapshot
    {
        .make(claude: ClaudeProviderSettings(
            usageDataSource: source,
            webExtrasEnabled: false,
            statusLineFeedEnabled: enabled,
            keychainAccessDisabled: keychainDisabled,
            statusLineStandaloneAllowed: standaloneAllowed,
            cookieSource: .off,
            manualCookieHeader: nil))
    }

    private func context(
        sourceMode: ProviderSourceMode = .auto,
        environment: [String: String] = [:],
        settings: ProviderSettingsSnapshot? = nil,
        selectedTokenAccountID: UUID? = nil) -> ProviderFetchContext
    {
        let browser = BrowserDetection(cacheTTL: 0)
        return ProviderFetchContext(
            runtime: .app,
            sourceMode: sourceMode,
            includeCredits: false,
            webTimeout: 1,
            webDebugDumpHTML: false,
            verbose: false,
            env: environment,
            settings: settings,
            fetcher: UsageFetcher(environment: environment),
            claudeFetcher: ClaudeUsageFetcher(browserDetection: browser),
            browserDetection: browser,
            selectedTokenAccountID: selectedTokenAccountID)
    }

    private func strategyIDs(_ context: ProviderFetchContext) async -> [String] {
        await ProviderDescriptorRegistry.descriptor(for: .claude)
            .fetchPlan.pipeline.resolveStrategies(context).map(\.id)
    }

    @Test
    func `standalone feed is limited to keychain disabled ambient Auto`() async {
        let eligible = self.context(settings: self.settings())
        #expect(await self.strategyIDs(eligible).first == "claude.statusline.standalone")

        let keychainEnabled = self.context(settings: self.settings(keychainDisabled: false))
        let keychainEnabledIDs = await self.strategyIDs(keychainEnabled)
        #expect(!keychainEnabledIDs.contains("claude.statusline.standalone"))

        let explicitCLI = self.context(sourceMode: .cli, settings: self.settings(source: .cli))
        let explicitCLIIDs = await self.strategyIDs(explicitCLI)
        #expect(!explicitCLIIDs.contains("claude.statusline.standalone"))

        let selectedToken = self.context(settings: self.settings(), selectedTokenAccountID: UUID())
        let selectedTokenIDs = await self.strategyIDs(selectedToken)
        #expect(!selectedTokenIDs.contains("claude.statusline.standalone"))

        let multiAccount = self.context(settings: self.settings(standaloneAllowed: false))
        let multiAccountIDs = await self.strategyIDs(multiAccount)
        #expect(!multiAccountIDs.contains("claude.statusline.standalone"))
    }

    @Test
    func `Admin API remains authoritative and never receives an observation overlay`() async {
        let env = [ClaudeAdminAPISettingsReader.adminAPIKeyEnvironmentKey: "sk-ant-admin-test"]
        let context = self.context(environment: env, settings: self.settings())
        #expect(await self.strategyIDs(context) == ["claude.admin-api"])
    }

    @Test
    func `keychain enabled mode falls through to the existing source plan`() async {
        let context = self.context(settings: self.settings(keychainDisabled: false))
        let ids = await self.strategyIDs(context)
        #expect(ids.first == "claude.oauth")
        #expect(!ids.contains("claude.statusline.standalone"))
    }

    @Test
    func `standalone mapping is anonymous and reduced fidelity`() async throws {
        let context = self.context(settings: self.settings())
        let observation = self.limits()
        let loader: ClaudeStatusLineFetchStrategy.ObservationLoader = { _ in observation }
        let outcome = await ClaudeStatusLineFetchStrategy.$observationLoaderOverrideForTesting
            .withValue(loader) {
                await ProviderDescriptorRegistry.descriptor(for: .claude).fetchOutcome(context: context)
            }
        let result = try outcome.result.get()
        #expect(result.sourceLabel == ClaudeStatusLineFeed.standaloneSourceLabel)
        #expect(result.usage.primary?.usedPercent == 40)
        #expect(result.usage.secondary?.usedPercent == 60)
        #expect(result.usage.tertiary == nil)
        #expect(result.usage.extraRateWindows == nil)
        #expect(result.usage.providerCost == nil)
        #expect(result.usage.identity == nil)
    }

    @Test
    func `weekly only observation stays in the weekly lane`() throws {
        let weeklyOnly = self.limits(fiveHour: nil, sevenDay: 55)
        let snapshot = try #require(ClaudeStatusLineDropStore.makeSnapshot(from: weeklyOnly))
        #expect(snapshot.primary == nil)
        #expect(snapshot.secondary?.usedPercent == 55)
        #expect(snapshot.secondary?.windowMinutes == 10080)
    }

    @Test
    func `five hour only observation stays in the primary lane`() throws {
        let fiveHourOnly = self.limits(fiveHour: 31, sevenDay: nil)
        let snapshot = try #require(ClaudeStatusLineDropStore.makeSnapshot(from: fiveHourOnly))
        #expect(snapshot.primary?.usedPercent == 31)
        #expect(snapshot.primary?.windowMinutes == 300)
        #expect(snapshot.secondary == nil)
    }

    @Test
    func `store accepts either window and rejects an empty observation`() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("codexbar-statusline-partial-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }

        let weeklyOnly = self.limits(fiveHour: nil, sevenDay: 27)
        try ClaudeStatusLineDropStore.write(weeklyOnly, applicationSupport: root)
        let loaded = try #require(ClaudeStatusLineDropStore.load(
            applicationSupport: root,
            expectedProfileID: weeklyOnly.profileID,
            now: self.now))
        #expect(loaded.fiveHour == nil)
        #expect(loaded.sevenDay?.usedPercent == 27)

        let empty = self.limits(fiveHour: nil, sevenDay: nil)
        #expect(throws: ClaudeStatusLineFileError.invalidObservation) {
            try ClaudeStatusLineDropStore.write(empty, applicationSupport: root)
        }
    }

    @Test
    func `stale future and wrong profile observations are absence`() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("codexbar-statusline-store-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        for (observation, expectedProfile) in [
            (self.limits(capturedAt: self.now.addingTimeInterval(-901)), "profile-a"),
            (self.limits(capturedAt: self.now.addingTimeInterval(301)), "profile-a"),
            (self.limits(profileID: "profile-a"), "profile-b"),
        ] {
            try ClaudeStatusLineDropStore.write(observation, applicationSupport: root)
            #expect(ClaudeStatusLineDropStore.load(
                applicationSupport: root,
                expectedProfileID: expectedProfile,
                now: self.now) == nil)
        }
    }

    @Test
    func `capture persists a minimal private allowlisted observation`() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("codexbar-statusline-capture-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let inputURL = root.appendingPathComponent("input.json")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        try Data(#"""
        {"rate_limits":{"five_hour":{"used_percentage":12,"resets_at":1800003600},
        "seven_day":{"used_percentage":34,"resets_at":1800086400}},
        "session_id":"secret","cwd":"/private/repo","cost":{"usd":20},"unknown":"discard"}
        """#.utf8).write(to: inputURL)
        let handle = try FileHandle(forReadingFrom: inputURL)
        CodexBarCLI.runClaudeStatusLineCapture(
            input: handle,
            environment: [:],
            applicationSupport: root,
            now: self.now)
        try handle.close()

        let profileID = ClaudeStatusLineProfile.identifier(environment: [:])
        let url = ClaudeStatusLineDropStore.observationURL(applicationSupport: root, profileID: profileID)
        let data = try Data(contentsOf: url)
        let text = try #require(String(data: data, encoding: .utf8))
        for forbidden in ["session_id", "secret", "cwd", "/private/repo", "cost", "unknown"] {
            #expect(!text.contains(forbidden))
        }
        let fileMode = try #require(
            (FileManager.default.attributesOfItem(atPath: url.path)[.posixPermissions] as? NSNumber)?.intValue)
        let directoryMode = try #require((FileManager.default.attributesOfItem(
            atPath: ClaudeStatusLineDropStore.directoryURL(applicationSupport: root).path)[
            .posixPermissions,
        ] as? NSNumber)?.intValue)
        #expect(fileMode == 0o600)
        #expect(directoryMode == 0o700)
    }

    @Test
    func `bounded stdin rejects oversized payloads`() throws {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("codexbar-statusline-large-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: url) }
        try Data(repeating: 0x20, count: 17).write(to: url)
        let handle = try FileHandle(forReadingFrom: url)
        #expect(try CodexBarCLI.readBoundedInput(handle, limit: 16) == nil)
        try handle.close()
    }

    @Test
    func `observation writer rejects a symlink destination`() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("codexbar-statusline-symlink-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let observation = self.limits()
        let directory = ClaudeStatusLineDropStore.directoryURL(applicationSupport: root)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let target = root.appendingPathComponent("target.json")
        try Data(#"{}"#.utf8).write(to: target)
        try FileManager.default.createSymbolicLink(
            at: ClaudeStatusLineDropStore.observationURL(
                applicationSupport: root,
                profileID: observation.profileID),
            withDestinationURL: target)
        #expect(throws: ClaudeStatusLineFileError.symbolicLink(
            ClaudeStatusLineDropStore.observationURL(
                applicationSupport: root,
                profileID: observation.profileID).path))
        {
            try ClaudeStatusLineDropStore.write(observation, applicationSupport: root)
        }
    }
}

@MainActor
struct ClaudeStatusLineSettingsTests {
    @Test
    func `opt in persists across relaunch and reaches the provider snapshot`() throws {
        let suite = "ClaudeStatusLineSettings-\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defaults.removePersistentDomain(forName: suite)
        defer { defaults.removePersistentDomain(forName: suite) }
        var settings = testSettingsStore(suiteName: suite, userDefaults: defaults)
        #expect(!settings.claudeStatusLineFeedEnabled)
        settings.claudeStatusLineFeedEnabled = true
        settings = testSettingsStore(suiteName: suite, userDefaults: defaults)
        #expect(settings.claudeStatusLineFeedEnabled)
        #expect(settings.claudeSettingsSnapshot(tokenOverride: nil).statusLineFeedEnabled)
    }

    @Test
    func `configured token accounts and claude swap disable standalone presentation`() {
        let tokenSettings = testSettingsStore(suiteName: "ClaudeStatusLineSettings-token")
        tokenSettings.debugDisableKeychainAccess = true
        tokenSettings.claudeStatusLineFeedEnabled = true
        tokenSettings.addTokenAccount(provider: .claude, label: "Token", token: "Bearer sk-ant-oat-test")
        #expect(!tokenSettings.claudeSettingsSnapshot(tokenOverride: nil).statusLineStandaloneAllowed)

        let swapSettings = testSettingsStore(suiteName: "ClaudeStatusLineSettings-swap")
        swapSettings.debugDisableKeychainAccess = true
        swapSettings.claudeStatusLineFeedEnabled = true
        swapSettings.claudeSwapEnabled = true
        #expect(!swapSettings.claudeSettingsSnapshot(tokenOverride: nil).statusLineStandaloneAllowed)
    }
}
