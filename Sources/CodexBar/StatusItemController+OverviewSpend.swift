import CodexBarCore
import Foundation

extension StatusItemController {
    func overviewSpendDashboardModel(
        providers: [UsageProvider],
        now: Date = Date()) -> SpendDashboardModel
    {
        guard self.settings.costUsageEnabled else {
            return SpendDashboardModel(requestedDays: self.settings.costUsageHistoryDays, groups: [])
        }
        let inputs = providers.compactMap { provider -> SpendDashboardModel.ProviderInput? in
            guard let snapshot = self.store.tokenSnapshotForCurrentProviderConfig(for: provider)?.snapshot else {
                return nil
            }
            return SpendDashboardModel.ProviderInput(
                provider: provider,
                displayName: self.store.metadata(for: provider).displayName,
                snapshot: snapshot)
        }
        return SpendDashboardModel.build(
            inputs: inputs,
            requestedDays: self.settings.costUsageHistoryDays,
            now: now,
            preferredCurrencyCode: self.settings.preferredCurrencyCode)
    }
}
