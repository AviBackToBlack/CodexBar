import { describe, expect, it } from "vitest";
import type { LocaleKey } from "../i18n/keys";
import type { ProviderDisplayDetail } from "../types/bridge";
import {
  providerCostPeriodTitle,
  providerDisplayDetailTitle,
} from "./providerLabels";

const translate = (key: LocaleKey) => `translated:${key}`;

describe("provider labels", () => {
  it("localizes Atlas Cloud display labels and preserves other provider titles", () => {
    const atlasBalance: ProviderDisplayDetail = {
      id: "atlascloud-available",
      title: "Available balance",
      value: "$95.50",
      secondaryValue: null,
      progress: null,
    };
    const other: ProviderDisplayDetail = {
      ...atlasBalance,
      id: "other",
      title: "Credits",
    };

    expect(providerDisplayDetailTitle(atlasBalance, translate)).toBe(
      "translated:AtlasCloudAvailableBalance",
    );
    expect(providerDisplayDetailTitle(other, translate)).toBe("Credits");
  });

  it("localizes Atlas Cloud balance period and preserves other periods", () => {
    expect(
      providerCostPeriodTitle("atlascloud", "Atlas Cloud balance", translate),
    ).toBe("translated:AtlasCloudBalance");
    expect(providerCostPeriodTitle("other", "This month", translate)).toBe("This month");
  });
});
