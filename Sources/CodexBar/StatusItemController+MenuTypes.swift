import AppKit
import CodexBarCore
import SwiftUI

extension StatusItemController {
    var selectedMenuProvider: ProviderInstanceID? {
        get { self.settings.selectedMenuProvider }
        set { self.settings.selectedMenuProvider = newValue }
    }

    var fallbackProvider: UsageProvider? {
        // Intentionally uses availability-filtered list: fallback activates when no provider
        // can actually work, ensuring at least a codex icon is always visible.
        self.store.enabledProviders().isEmpty ? .codex : nil
    }
}

extension ProviderSwitcherSelection {
    var provider: UsageProvider? {
        switch self {
        case .overview:
            nil
        case let .provider(instanceID):
            instanceID.firstPartyProvider
        }
    }

    var instanceID: ProviderInstanceID? {
        switch self {
        case .overview: nil
        case let .provider(instanceID): instanceID
        }
    }
}

struct OverviewSpendSummary: Equatable {
    let primarySpendText: String
    let coverageText: String
    let tokenText: String?
    let isPartial: Bool

    init(model: SpendDashboardModel, providerCount: Int) {
        let providerCount = max(0, providerCount)
        let knownCostCount = model.groups.reduce(0) { $0 + $1.pricedProviderCount }
        let knownTokenRows = model.groups.flatMap(\.providers).compactMap(\.totalTokens)
        let knownTokens = Self.safeTokenSum(knownTokenRows)
        let tokenCoverageIsComplete = knownTokenRows.count == providerCount &&
            model.groups.allSatisfy { $0.totalTokens != nil }
        self.isPartial = knownCostCount < providerCount || model.groups.contains { $0.totalCost == nil }

        let spendTexts = model.groups.compactMap { group -> String? in
            guard let cost = group.totalCost ?? Self.safeCostSum(group.providers.compactMap(\.totalCost)) else {
                return nil
            }
            let formatted = UsageFormatter.currencyString(cost, currencyCode: group.currencyCode)
            let groupIsPartial = group.totalCost == nil || knownCostCount < providerCount
            return groupIsPartial ? "~\(formatted)" : formatted
        }
        self.primarySpendText = spendTexts.isEmpty ? L("Spend unavailable") : spendTexts.joined(separator: " · ")
        self.coverageText = "\(codexBarLocalizedInteger(knownCostCount)) / " +
            "\(codexBarLocalizedInteger(providerCount)) \(L("Providers"))"
        self.tokenText = knownTokens.map {
            let formatted = ShareStatsFormatting.compactCount($0)
            let value = tokenCoverageIsComplete ? formatted : "~\(formatted)"
            return L("%@ tokens", value)
        }
    }

    private static func safeTokenSum(_ values: [Int]) -> Int? {
        var total = 0
        for value in values {
            let result = total.addingReportingOverflow(value)
            guard !result.overflow else { return nil }
            total = result.partialValue
        }
        return values.isEmpty ? nil : total
    }

    private static func safeCostSum(_ values: [Double]) -> Double? {
        guard !values.isEmpty else { return nil }
        var total = 0.0
        for value in values {
            guard value.isFinite else { return nil }
            let result = total + value
            guard result.isFinite else { return nil }
            total = result
        }
        return total
    }
}

struct OverviewSpendSummaryCardView: View {
    let summary: OverviewSpendSummary
    let days: Int
    let width: CGFloat

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            HStack(spacing: 6) {
                Text(L("Usage & Spend"))
                    .font(.headline.weight(.semibold))
                Text("·")
                Text(spendDashboardDayRangeText(self.days))
            }
            .foregroundStyle(.secondary)

            Text(self.summary.primarySpendText)
                .font(.system(.title2, design: .rounded, weight: .bold))
                .monospacedDigit()
                .lineLimit(2)

            HStack(spacing: 8) {
                Text(self.summary.coverageText)
                if let tokenText = self.summary.tokenText {
                    Text("·")
                    Text(tokenText)
                }
            }
            .font(.caption)
            .foregroundStyle(.secondary)
            .lineLimit(1)
        }
        .padding(.horizontal, UsageMenuCardLayout.horizontalPadding)
        .padding(.vertical, 10)
        .frame(width: self.width, alignment: .leading)
        .background {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(Color.accentColor.opacity(0.08))
                .padding(.horizontal, 6)
        }
    }
}

struct OverviewMenuCardRowView: View {
    static let showsSectionDividers = false

    let model: UsageMenuCardView.Model
    let storageText: String?
    let width: CGFloat
    @Environment(\.menuItemHighlighted) private var isHighlighted

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            UsageMenuCardHeaderSectionView(
                model: self.model,
                showDivider: Self.showsSectionDividers && self.hasUsageBlock,
                width: self.width)
            if self.hasUsageBlock {
                UsageMenuCardUsageSectionView(
                    model: self.model,
                    showBottomDivider: false,
                    bottomPadding: 6,
                    width: self.width,
                    showsSectionDividers: Self.showsSectionDividers)
            }
            if let storageText {
                HStack(alignment: .firstTextBaseline, spacing: 4) {
                    Text("\(L("Storage")):")
                        .font(.footnote)
                        .foregroundStyle(MenuHighlightStyle.secondary(self.isHighlighted))
                    Text(storageText)
                        .font(.footnote)
                        .foregroundStyle(MenuHighlightStyle.secondary(self.isHighlighted))
                        .lineLimit(1)
                    Spacer()
                }
                .padding(.horizontal, UsageMenuCardLayout.horizontalPadding)
                .padding(.top, self.hasUsageBlock ? 0 : 8)
                .padding(.bottom, 6)
                .frame(width: self.width, alignment: .leading)
            }
        }
        .frame(width: self.width, alignment: .leading)
    }

    private var hasUsageBlock: Bool {
        self.model.hasUsageContent
    }
}

struct OpenAIWebMenuItems {
    let hasUsageBreakdown: Bool
    let hasCreditsHistory: Bool
    let hasCostHistory: Bool
    let canShowBuyCredits: Bool
}

struct TokenAccountMenuDisplay: Equatable {
    let provider: UsageProvider
    let accounts: [ProviderTokenAccount]
    let snapshots: [TokenAccountUsageSnapshot]
    let activeIndex: Int
    let layout: MultiAccountMenuLayout

    var showAll: Bool {
        self.layout == .stacked
    }

    var showSwitcher: Bool {
        self.layout == .segmented
    }

    static func == (lhs: TokenAccountMenuDisplay, rhs: TokenAccountMenuDisplay) -> Bool {
        lhs.provider == rhs.provider &&
            lhs.accountIdentity == rhs.accountIdentity &&
            lhs.activeIndex == rhs.activeIndex &&
            lhs.layout == rhs.layout &&
            lhs.snapshotIdentity == rhs.snapshotIdentity
    }

    private var accountIdentity: [AccountIdentity] {
        self.accounts.map { account in
            AccountIdentity(
                id: account.id,
                label: account.label,
                externalIdentifier: account.externalIdentifier,
                usageScope: account.usageScope,
                organizationID: account.organizationID,
                workspaceID: account.workspaceID)
        }
    }

    private var snapshotIdentity: [SnapshotIdentity] {
        self.snapshots.map { snapshot in
            SnapshotIdentity(
                id: snapshot.id,
                hasSnapshot: snapshot.snapshot != nil,
                error: snapshot.error,
                sourceLabel: snapshot.sourceLabel)
        }
    }

    private struct AccountIdentity: Equatable {
        let id: UUID
        let label: String
        let externalIdentifier: String?
        let usageScope: String?
        let organizationID: String?
        let workspaceID: String?
    }

    private struct SnapshotIdentity: Equatable {
        let id: UUID
        let hasSnapshot: Bool
        let error: String?
        let sourceLabel: String?
    }
}

struct CodexAccountMenuDisplay: Equatable {
    let accounts: [CodexVisibleAccount]
    let snapshots: [CodexAccountUsageSnapshot]
    let activeVisibleAccountID: String?
    let layout: MultiAccountMenuLayout

    var showAll: Bool {
        self.layout == .stacked
    }

    var showSwitcher: Bool {
        self.layout == .segmented
    }

    var workspaceSections: [CodexAccountWorkspaceSection] {
        self.accounts.codexWorkspaceSections()
    }

    var showsWorkspaceGroups: Bool {
        Set(self.workspaceSections.map(\.title)).count > 1
    }

    static func == (lhs: CodexAccountMenuDisplay, rhs: CodexAccountMenuDisplay) -> Bool {
        lhs.accounts == rhs.accounts &&
            lhs.activeVisibleAccountID == rhs.activeVisibleAccountID &&
            lhs.layout == rhs.layout &&
            lhs.snapshotIdentity == rhs.snapshotIdentity
    }

    private var snapshotIdentity: [SnapshotIdentity] {
        self.snapshots.map { snapshot in
            SnapshotIdentity(
                id: snapshot.id,
                hasSnapshot: snapshot.snapshot != nil,
                error: snapshot.error,
                sourceLabel: snapshot.sourceLabel)
        }
    }

    private struct SnapshotIdentity: Equatable {
        let id: String
        let hasSnapshot: Bool
        let error: String?
        let sourceLabel: String?
    }
}
