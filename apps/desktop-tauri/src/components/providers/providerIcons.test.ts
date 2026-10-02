import { describe, expect, it } from "vitest";
import { TEST_PROVIDER_CATALOG } from "../../test/providerCatalog";
import { PROVIDER_ICON_REGISTRY } from "./providerIcons";

describe("provider icon registry", () => {
  it("has explicit icon metadata for every provider in the catalog", () => {
    for (const [id] of TEST_PROVIDER_CATALOG) {
      expect(PROVIDER_ICON_REGISTRY[id], id).toBeDefined();
    }
  });

  it("ships the upstream Atlas Cloud glyph tinted by the brand color", () => {
    const svg = PROVIDER_ICON_REGISTRY.atlascloud.svgPath;
    expect(svg).toContain("<svg");
    expect(svg).toContain('fill="currentColor"');
    expect(svg).not.toContain('fill="#000"');
  });

  it("does not expose the retired Crof provider", () => {
    expect(PROVIDER_ICON_REGISTRY).not.toHaveProperty("crof");
  });
});
