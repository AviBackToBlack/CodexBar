/**
 * Preferred display-currency formatting for the web UI.
 *
 * Rust owns the currency model (supported codes, fallback rates, conversion,
 * rate sanitizing — see `codexbar::currency`); `get_currency_rates` returns
 * the merged, sanitized table plus the normalized preference, so this module
 * only renders strings through `Intl`.
 */

export interface CurrencyRatesSnapshot {
  rates: Record<string, number>;
  supportedCodes: readonly string[];
  preferredCode: string;
}

function formatOriginal(amount: number, code: string, symbol?: string | null): string {
  if (symbol) return `${symbol}${amount.toFixed(2)}`;
  if (!/^[A-Z]{3}$/.test(code)) return `${amount.toFixed(2)} ${code}`;
  try {
    return new Intl.NumberFormat("en-US", { style: "currency", currency: code }).format(amount);
  } catch {
    return `${amount.toFixed(2)} ${code}`;
  }
}

export function formatDisplayCurrency(
  amount: number | null | undefined,
  sourceCode: string,
  preferredCode: string,
  rates: Record<string, number>,
  sourceSymbol?: string | null,
): string {
  if (amount == null || !Number.isFinite(amount)) return "—";
  const trimmedSource = sourceCode.trim();
  const source = trimmedSource.toUpperCase();
  const sourceLabel = /^[A-Za-z]{3}$/.test(trimmedSource) ? source : trimmedSource;
  const preferred = preferredCode.trim().toUpperCase() || "AUTO";
  if (preferred === "AUTO") return formatOriginal(amount, sourceLabel, sourceSymbol);
  if (preferred === source) return formatOriginal(amount, sourceLabel, sourceSymbol);
  const sourceRate = source === "USD" ? 1 : rates[source];
  const targetRate = preferred === "USD" ? 1 : rates[preferred];
  const usable =
    Number.isFinite(amount) &&
    Number.isFinite(sourceRate) && sourceRate! > 0 &&
    Number.isFinite(targetRate) && targetRate! > 0;
  if (!usable) return formatOriginal(amount, sourceLabel, sourceSymbol);
  const converted = (amount / sourceRate!) * targetRate!;
  if (!Number.isFinite(converted)) return formatOriginal(amount, sourceLabel, sourceSymbol);
  try {
    return new Intl.NumberFormat(undefined, {
      style: "currency",
      currency: preferred,
      maximumFractionDigits: 2,
    }).format(converted);
  } catch {
    return `${converted.toFixed(2)} ${preferred}`;
  }
}

export function sumDisplayCurrencyAmounts(
  rows: Array<{ amount: number | null | undefined; currency: string }>,
  preferredCode: string,
  rates: Record<string, number>,
): { total: number | null; included: number; considered: number } {
  const target = preferredCode.trim().toUpperCase() || "AUTO";
  let total = 0;
  let included = 0;
  for (const row of rows) {
    if (row.amount == null || !Number.isFinite(row.amount)) continue;
    let amount: number | null;
    if (target === "AUTO") {
      amount = row.currency.trim().toUpperCase() === "USD" ? row.amount : null;
    } else {
      const source = row.currency.trim().toUpperCase() || "USD";
      const sourceRate = source === "USD" ? 1 : rates[source];
      const targetRate = target === "USD" ? 1 : rates[target];
      const usable =
        Number.isFinite(sourceRate) && sourceRate! > 0 &&
        Number.isFinite(targetRate) && targetRate! > 0;
      amount = usable ? (row.amount / sourceRate!) * targetRate! : null;
      if (amount != null && !Number.isFinite(amount)) amount = null;
    }
    if (amount == null) continue;
    total += amount;
    included += 1;
  }
  return { total: included > 0 && Number.isFinite(total) ? total : null, included, considered: rows.length };
}
