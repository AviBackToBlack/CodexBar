import { describe, expect, it } from "vitest";
import { hideOpenAiApiProjectId } from "./openAiApiIdentity";

describe("hideOpenAiApiProjectId", () => {
  it("masks only the project id in OpenAI identity labels", () => {
    expect(hideOpenAiApiProjectId("Admin API: proj-private", true)).toBe("Admin API: ••••");
    expect(hideOpenAiApiProjectId("Project: proj-private", true)).toBe("Project: ••••");
  });

  it("preserves visible identity text when privacy is off", () => {
    expect(hideOpenAiApiProjectId("Admin API: proj-private", false)).toBe("Admin API: proj-private");
  });

  it("does not alter unrelated labels", () => {
    expect(hideOpenAiApiProjectId("Admin API", true)).toBe("Admin API");
    expect(hideOpenAiApiProjectId("gpt-5", true)).toBe("gpt-5");
    expect(hideOpenAiApiProjectId(null, true)).toBeNull();
  });
});
