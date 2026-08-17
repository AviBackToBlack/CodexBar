import Foundation
import Testing
@testable import CodexBarCore

struct ClaudeStatusLinePayloadTests {
    private let now = Date(timeIntervalSince1970: 1_800_000_000)

    private func parse(_ json: String) -> ClaudeStatusLineRateLimits? {
        ClaudeStatusLinePayloadParser.parseOfficialPayload(
            Data(json.utf8),
            capturedAt: self.now,
            environment: [ClaudeConfigPaths.configDirectoryEnvironmentKey: "/tmp/profile-a"])
    }

    @Test
    func `reads independently available official windows`() throws {
        let limits = try #require(self.parse(#"""
        {
          "rate_limits": {
            "five_hour": {"used_percentage": 42.5, "resets_at": 1800003600, "unknown": "ignored"},
            "seven_day": {"used_percentage": 63, "resets_at": "2027-01-22T00:00:00Z"}
          },
          "session_id": "must-not-persist",
          "cwd": "/private/project",
          "cost": {"total_cost_usd": 99},
          "model": {"display_name": "private"}
        }
        """#))
        #expect(limits.fiveHour?.usedPercent == 42.5)
        #expect(limits.sevenDay?.usedPercent == 63)
        #expect(limits.fiveHour?.resetsAt == Date(timeIntervalSince1970: 1_800_003_600))
        #expect(limits.sevenDay?.resetsAt == nil)
        #expect(limits.capturedAt == self.now)
        #expect(limits.profileID == ClaudeStatusLineProfile.identifier(environment: [
            ClaudeConfigPaths.configDirectoryEnvironmentKey: "/tmp/profile-a",
        ]))
    }

    @Test
    func `schema drift with no valid percentage fails soft`() {
        let cases = [
            "not json",
            "{}",
            #"{"rate_limits":[]}"#,
            #"{"rate_limits":{"five_hour":{"utilization":12,"resets_at":1800003600}}}"#,
            #"{"rate_limits":{"five_hour":{"used_percentage":"12","resets_at":1800003600}}}"#,
            #"{"rate_limits":{"five_hour":{"used_percentage":true,"resets_at":1800003600}}}"#,
            #"{"rate_limits":{"five_hour":{"used_percentage":101,"resets_at":1800003600}}}"#,
        ]
        for json in cases {
            #expect(self.parse(json) == nil, "drifted payload must be absence: \(json)")
        }
    }

    @Test
    func `missing window and reset preserve a valid percentage`() throws {
        let limits = try #require(self.parse(#"{"rate_limits":{"five_hour":{"used_percentage":9}}}"#))
        #expect(limits.fiveHour?.usedPercent == 9)
        #expect(limits.fiveHour?.resetsAt == nil)
        #expect(limits.sevenDay == nil)
    }

    @Test(arguments: [
        #""later""#,
        "true",
        "946684799",
        "4102444801",
    ])
    func `invalid reset metadata preserves the weekly percentage`(resetJSON: String) throws {
        let limits = try #require(self.parse(
            #"{"rate_limits":{"seven_day":{"used_percentage":44,"resets_at":\#(resetJSON)}}}"#))
        #expect(limits.fiveHour == nil)
        #expect(limits.sevenDay?.usedPercent == 44)
        #expect(limits.sevenDay?.resetsAt == nil)
    }

    @Test
    func `oversized input is absence`() {
        let oversized = Data(repeating: 0x20, count: ClaudeStatusLineFeed.maximumInputBytes + 1)
        #expect(ClaudeStatusLinePayloadParser.parseOfficialPayload(oversized) == nil)
    }
}
