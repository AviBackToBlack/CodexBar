import { describe, expect, it } from "vitest";
import { hasSuccessfulClaudeCliQuota } from "./claudeAccountActions";
import type { ProviderUsageSnapshot } from "../types/bridge";

function provider(
  overrides: Partial<ProviderUsageSnapshot> = {},
): Pick<
  ProviderUsageSnapshot,
  "providerId" | "sourceLabel" | "error" | "errorState" | "primary" | "secondary"
> {
  return {
    providerId: "claude",
    sourceLabel: "Claude CLI",
    error: null,
    errorState: "ready",
    primary: {
      usedPercent: 25,
      isExhausted: false,
      resetsAt: null,
      resetDescription: null,
      isInformational: false,
    },
    secondary: null,
    ...overrides,
  };
}

describe("hasSuccessfulClaudeCliQuota", () => {
  it("allows account actions without an identity field", () => {
    expect(hasSuccessfulClaudeCliQuota(provider())).toBe(true);
  });

  it("does not treat retained, failed, or non-CLI data as account proof", () => {
    expect(hasSuccessfulClaudeCliQuota(provider({ sourceLabel: "OAuth" }))).toBe(false);
    expect(hasSuccessfulClaudeCliQuota(provider({ error: "timed out" }))).toBe(false);
    expect(hasSuccessfulClaudeCliQuota(provider({ errorState: "unknown" }))).toBe(false);
    expect(
      hasSuccessfulClaudeCliQuota(
        provider({
          primary: {
            usedPercent: 25,
            isExhausted: false,
            resetsAt: null,
            resetDescription: "Status",
            isInformational: true,
          },
        }),
      ),
    ).toBe(false);
  });
});
