import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { LocaleKey } from "../../../../i18n/keys";
import { IdentitySection } from "./IdentitySection";

type IdentityDetails = Parameters<typeof IdentitySection>[0]["provider"];

function detail(overrides: Partial<IdentityDetails> = {}): IdentityDetails {
  return {
    id: "openaiapi",
    displayName: "OpenAI API",
    organization: "Project: proj-private",
    plan: "Admin API: proj-private",
    email: null,
    authType: null,
    sourceLabel: null,
    ...overrides,
  };
}

const t = (key: LocaleKey) => key;

describe("IdentitySection OpenAI project privacy", () => {
  it("masks project ids in account and plan rows when privacy is enabled", () => {
    render(
      <IdentitySection
        provider={detail()}
        subtitle=""
        t={t}
        hidePersonalInfo
      />,
    );

    expect(screen.getByText("Project: ••••")).toBeInTheDocument();
    expect(screen.getByText("Admin API: ••••")).toBeInTheDocument();
    expect(screen.queryByText("proj-private")).toBeNull();
  });

  it("keeps project ids visible when privacy is disabled", () => {
    render(<IdentitySection provider={detail()} subtitle="" t={t} />);

    expect(screen.getAllByText(/proj-private/)).toHaveLength(2);
  });
});
