import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { TEST_PROVIDER_CATALOG } from "../../test/providerCatalog";
import { PROVIDER_ICON_REGISTRY } from "./providerIcons";

// Repository root, from apps/desktop-tauri/src/components/providers.
const REPO_ROOT = import.meta.dirname + "/../../../../../";

/**
 * The `ProviderId::X => "#RRGGBB"` rows of the Rust `brand_color` table in
 * `rust/src/core/provider.rs`, keyed by the lowercased variant name (which
 * is the registry id for every provider).
 */
function rustBrandColors(): Map<string, string> {
  const source = readFileSync(REPO_ROOT + "rust/src/core/provider.rs", "utf8");
  const start = source.indexOf("pub fn brand_color(id: ProviderId)");
  const end = source.indexOf("\n}\n", start);
  const rows = new Map<string, string>();
  if (start < 0 || end < 0) return rows;
  for (const match of source
    .slice(start, end)
    .matchAll(/ProviderId::(\w+) => "(#[0-9A-Fa-f]{6})"/g)) {
    rows.set(match[1].toLowerCase(), match[2].toLowerCase());
  }
  return rows;
}

/** `--chart-<id>: rgb(r, g, b);` provider tokens from styles.css, as hex. */
function chartTokens(): Array<[string, string]> {
  const css = readFileSync(import.meta.dirname + "/../../styles.css", "utf8");
  const tokens: Array<[string, string]> = [];
  for (const match of css.matchAll(
    /--chart-([a-z0-9]+): rgb\((\d+), (\d+), (\d+)\);/g,
  )) {
    const hex = [match[2], match[3], match[4]]
      .map((channel) => Number(channel).toString(16).padStart(2, "0"))
      .join("");
    tokens.push([match[1], `#${hex}`]);
  }
  return tokens;
}

// Upstream 0.70.0 palette audit (#4075, docs/provider-palette.md): the 16
// accents it adopted. The Rust side pins the same values and checks their
// contrast against the previous Windows accents.
const ADOPTED_ACCENTS: Array<[string, string]> = [
  ["abacus", "#814ee8"],
  ["amp", "#f34e3f"],
  ["augment", "#1aa049"],
  ["bedrock", "#01a88d"],
  ["clinepass", "#5487c8"],
  ["codebuff", "#00ff95"],
  ["commandcode", "#8c4edd"],
  ["cursor", "#f54e00"],
  ["deepseek", "#4d6bfe"],
  ["devin", "#317cff"],
  ["kiro", "#9046ff"],
  ["longcat", "#29e154"],
  ["mistral", "#ff5229"],
  ["neuralwatt", "#d55934"],
  ["sub2api", "#14b8a6"],
  ["venice", "#3c8fdd"],
];

describe("provider icon registry", () => {
  it("has explicit icon metadata for every provider in the catalog", () => {
    for (const [id] of TEST_PROVIDER_CATALOG) {
      expect(PROVIDER_ICON_REGISTRY[id], id).toBeDefined();
    }
  });

  it("does not expose the retired Crof provider", () => {
    expect(PROVIDER_ICON_REGISTRY).not.toHaveProperty("crof");
  });

  it("uses the accents adopted by the upstream 0.70.0 palette audit", () => {
    for (const [id, color] of ADOPTED_ACCENTS) {
      expect(PROVIDER_ICON_REGISTRY[id]?.brandColor.toLowerCase(), id).toBe(color);
    }
  });

  it("matches the Rust brand_color table for every provider", () => {
    const rust = rustBrandColors();
    expect(rust.size).toBeGreaterThan(0);
    expect([...rust.keys()].sort()).toEqual(Object.keys(PROVIDER_ICON_REGISTRY).sort());
    for (const [id, color] of rust) {
      expect(PROVIDER_ICON_REGISTRY[id]?.brandColor.toLowerCase(), id).toBe(color);
    }
  });

  it("keeps provider chart tokens on the brand color", () => {
    // Grok's chart token has been pure black since before the registry
    // existed (xAI shares it), so it is the one intentional exception.
    const checked = chartTokens().filter(
      ([id]) => id !== "grok" && PROVIDER_ICON_REGISTRY[id] !== undefined,
    );
    expect(checked.map(([id]) => id)).toEqual(
      expect.arrayContaining(["amp", "augment", "codebuff", "cursor", "deepseek", "kiro", "mistral"]),
    );
    for (const [id, color] of checked) {
      expect(color, `--chart-${id}`).toBe(PROVIDER_ICON_REGISTRY[id].brandColor.toLowerCase());
    }
  });
});
