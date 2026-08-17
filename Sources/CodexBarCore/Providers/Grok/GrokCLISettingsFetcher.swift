import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

/// Reads the Grok CLI settings envelope. Plan names live here (`subscription_tier_display`),
/// not on `/v1/billing?format=credits`.
enum GrokCLISettingsFetcher {
    static let defaultEndpoint = URL(string: "https://cli-chat-proxy.grok.com/v1/settings")!
    static let requestTimeoutSeconds: TimeInterval = 2

    private final class Cache: @unchecked Sendable {
        private let lock = NSLock()
        private var entries: [String: String] = [:]

        func remember(_ tier: String?, key: String) {
            guard let tier else { return }
            self.lock.lock()
            self.entries[key] = tier
            self.lock.unlock()
        }

        func tier(for key: String) -> String? {
            self.lock.lock()
            defer { self.lock.unlock() }
            return self.entries[key]
        }

        func reset() {
            self.lock.lock()
            self.entries = [:]
            self.lock.unlock()
        }
    }

    private static let cache = Cache()

    static func endpoint(fromBilling billing: URL) -> URL {
        var components = URLComponents(url: billing, resolvingAgainstBaseURL: false)
        components?.path = "/v1/settings"
        components?.query = nil
        return components?.url ?? self.defaultEndpoint
    }

    static func subscriptionTierDisplay(
        credentials: GrokCredentials,
        session transport: any ProviderHTTPTransport,
        endpoint: URL = Self.defaultEndpoint) async throws -> String?
    {
        guard !credentials.isExpired else { return nil }

        var request = URLRequest(url: endpoint)
        request.httpMethod = "GET"
        request.timeoutInterval = Self.requestTimeoutSeconds
        request.setValue("Bearer \(credentials.accessToken)", forHTTPHeaderField: "Authorization")
        request.setValue("xai-grok-cli", forHTTPHeaderField: "x-xai-token-auth")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue("CodexBar", forHTTPHeaderField: "User-Agent")

        let response: ProviderHTTPResponse
        do {
            response = try await transport.response(for: request)
        } catch is CancellationError {
            throw CancellationError()
        } catch let error as URLError where error.code == .cancelled {
            throw error
        } catch {
            return nil
        }
        guard response.statusCode == 200 else { return nil }
        return self.parse(response.data)
    }

    static func remember(_ tier: String?, for credentials: GrokCredentials) {
        guard let key = self.cacheKey(for: credentials) else { return }
        self.cache.remember(tier, key: key)
    }

    static func cachedTier(for credentials: GrokCredentials) -> String? {
        guard let key = self.cacheKey(for: credentials) else { return nil }
        return self.cache.tier(for: key)
    }

    static func resetCacheForTesting() {
        self.cache.reset()
    }

    static func cacheKey(for credentials: GrokCredentials) -> String? {
        let identity = credentials.userId ?? credentials.email
        guard let identity else { return nil }
        if credentials.isTeamPrincipal {
            let teamID = credentials.teamId?.trimmingCharacters(in: .whitespacesAndNewlines)
            return "team:\(teamID?.isEmpty == false ? teamID! : "_"):\(identity)"
        }
        let principal = credentials.principalType?
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
        return "\(principal?.isEmpty == false ? principal! : "user"):\(identity)"
    }

    static func parse(_ data: Data) -> String? {
        struct SettingsResponse: Decodable {
            let subscriptionTierDisplay: String?

            enum CodingKeys: String, CodingKey {
                case subscriptionTierDisplay = "subscription_tier_display"
            }
        }

        guard let response = try? JSONDecoder().decode(SettingsResponse.self, from: data) else {
            return nil
        }
        return GrokPlan.displayName(from: response.subscriptionTierDisplay)
    }
}
