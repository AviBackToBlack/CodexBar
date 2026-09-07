import { describe, expect, it } from "vitest";
import type { ProviderUsageSnapshot } from "../types/bridge";
import { hasSuccessfulClaudeCliQuota } from "./claudeAccountActions";

function provider(
  overrides: Partial<ProviderUsageSnapshot> = {},
): Pick<
  ProviderUsageSnapshot,
  | "providerId"
  | "sourceLabel"
  | "hasSuccessfulClaudeCliQuota"
  | "error"
  | "primary"
  | "secondary"
> {
  return {
    providerId: "claude",
    sourceLabel: "Claude CLI",
    hasSuccessfulClaudeCliQuota: true,
    error: null,
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
    expect(hasSuccessfulClaudeCliQuota(provider({ hasSuccessfulClaudeCliQuota: false }))).toBe(
      false,
    );
    expect(hasSuccessfulClaudeCliQuota(provider({ error: "timed out" }))).toBe(false);
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
