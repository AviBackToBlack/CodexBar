import CodexBarCore
import Foundation

extension CodexBarCLI {
    static func runClaudeStatusLineCapture(
        input: FileHandle = .standardInput,
        environment: [String: String] = ProcessInfo.processInfo.environment,
        applicationSupport: URL? = nil,
        now: Date = Date())
    {
        guard let data = try? self.readBoundedInput(input, limit: ClaudeStatusLineFeed.maximumInputBytes),
              let observation = ClaudeStatusLinePayloadParser.parseOfficialPayload(
                  data,
                  capturedAt: now,
                  environment: environment),
              let support = applicationSupport ?? FileManager.default.urls(
                  for: .applicationSupportDirectory,
                  in: .userDomainMask).first
        else { return }
        try? ClaudeStatusLineDropStore.write(observation, applicationSupport: support)
        // Intentionally no stdout: Claude Code renders stdout as the status line.
    }

    static func readBoundedInput(_ input: FileHandle, limit: Int) throws -> Data? {
        var result = Data()
        while let chunk = try input.read(upToCount: min(64 * 1024, limit + 1 - result.count)), !chunk.isEmpty {
            result.append(chunk)
            if result.count > limit {
                return nil
            }
        }
        return result
    }
}
