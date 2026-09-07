import type { ProviderUsageSnapshot, RateWindowSnapshot } from "../types/bridge";

/**
 * A Claude CLI quota response proves that account actions are useful even
 * when Claude does not expose an email or other account identity.
 *
 * Keep this deliberately narrower than "has Claude data": retained data,
 * web/OAuth data, and failed refreshes must not authenticate an account.
 */
export function hasSuccessfulClaudeCliQuota(
  provider: Pick<
    ProviderUsageSnapshot,
    "providerId" | "sourceLabel" | "error" | "primary" | "secondary"
  >,
): boolean {
  if (
    provider.providerId !== "claude" ||
    provider.error !== null ||
    !isClaudeCliSource(provider.sourceLabel)
  ) {
    return false;
  }

  return hasQuotaWindow(provider.primary) || hasQuotaWindow(provider.secondary);
}

function isClaudeCliSource(sourceLabel: string): boolean {
  const normalized = sourceLabel.trim().toLowerCase();
  return (
    normalized === "claude" ||
    normalized === "cli" ||
    normalized.includes("claude cli") ||
    normalized.includes("claude code")
  );
}

function hasQuotaWindow(window: RateWindowSnapshot | null): boolean {
  return window !== null && !window.isInformational && Number.isFinite(window.usedPercent);
}
