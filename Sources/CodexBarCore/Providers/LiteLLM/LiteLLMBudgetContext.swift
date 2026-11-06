import Foundation

/// Budget identity survives formatting, persistence, and snapshot enrichment.
public struct LiteLLMBudgetContext: Codable, Sendable, Equatable {
    public enum Source: String, Codable, Sendable {
        case key
        case spend
    }

    public enum Window: String, Codable, Sendable {
        case key
        case personal
        case team

        var title: String {
            switch self {
            case .key: "Key budget"
            case .personal: "Personal budget"
            case .team: "Team budget"
            }
        }
    }

    public let source: Source
    public let primary: Window?
    public let secondary: Window?
    public let tertiary: Window?

    private enum CodingKeys: String, CodingKey {
        case source = "litellm.budget.source"
        case primary
        case secondary
        case tertiary
    }

    public init(source: Source, primary: Window?, secondary: Window?, tertiary: Window?) {
        self.source = source
        self.primary = primary
        self.secondary = secondary
        self.tertiary = tertiary
    }
}
