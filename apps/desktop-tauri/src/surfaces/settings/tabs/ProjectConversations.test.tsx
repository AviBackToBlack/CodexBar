import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import ProjectConversations, {
  CONVERSATION_INITIAL_ROWS,
  CONVERSATION_MAX_ROWS,
} from "./ProjectConversations";
import type { CodexWorkspacesSessionUsage } from "../../../types/bridge";
import type { LocaleKey } from "../../../i18n/keys";

const t = (key: LocaleKey) => key;

function session(index: number, unknownTokens = 0): CodexWorkspacesSessionUsage {
  return {
    id: `session-${index}`,
    projectId: "project-a",
    displayTitle: `Thread ${index}`,
    cwd: null,
    startedAt: null,
    latestActivity: null,
    totals: { inputTokens: 10, cachedInputTokens: 0, outputTokens: 0, totalTokens: 10 },
    costEstimate: { knownUsd: 100 - index, unknownTokens },
    topModel: null,
  };
}

describe("ProjectConversations", () => {
  it("shows ranked rows in the given order with their rank", () => {
    render(<ProjectConversations conversations={[session(1), session(2, 5)]} t={t} />);

    expect(screen.getByText("#1")).toBeTruthy();
    expect(screen.getByText("#2")).toBeTruthy();
    expect(screen.getByText("Thread 1").getAttribute("title")).toBe("Thread 1");
    expect(screen.getByText(/\$99\.00/)).toBeTruthy();
    expect(screen.getByText(/~\$98\.00/)).toBeTruthy();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("shows the top rows first and expands up to the display limit", () => {
    const conversations = Array.from({ length: CONVERSATION_MAX_ROWS + 5 }, (_, i) => session(i + 1));
    render(<ProjectConversations conversations={conversations} t={t} />);

    expect(screen.getAllByText(/^#\d+$/)).toHaveLength(CONVERSATION_INITIAL_ROWS);
    fireEvent.click(screen.getByRole("button", { name: `UsageSpendShowAll (${CONVERSATION_MAX_ROWS})` }));
    expect(screen.getAllByText(/^#\d+$/)).toHaveLength(CONVERSATION_MAX_ROWS);
    expect(screen.queryByText(`Thread ${CONVERSATION_MAX_ROWS + 1}`)).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "UsageSpendShowLess" }));
    expect(screen.getAllByText(/^#\d+$/)).toHaveLength(CONVERSATION_INITIAL_ROWS);
  });
});
