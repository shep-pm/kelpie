import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentView } from "@/bindings";

const ok = <T>(data: T) => ({ status: "ok" as const, data });

const agentGet = vi.fn<(name: string) => Promise<unknown>>();

vi.mock("@/bindings", () => ({
  commands: { agentGet: (name: string) => agentGet(name) },
}));

import { useAgents } from "./store";

const view = (name: string): AgentView => ({
  name,
  exists: true,
  frontmatter: {
    extends: null,
    role: "pm",
    harness: null,
    model: null,
    effort: null,
    body: null,
    paths: null,
    second_look: null,
  },
  body: `${name}'s body`,
  problem: null,
  resolved: null,
});

beforeEach(() => useAgents.getState().reset());
afterEach(() => vi.clearAllMocks());

describe("the agents store", () => {
  it("an agent read late does not replace the one opened after it", async () => {
    let answerA: (value: unknown) => void = () => {};
    agentGet.mockImplementation((name) =>
      name === "a"
        ? new Promise((resolve) => {
            answerA = resolve;
          })
        : Promise.resolve(ok(view("b"))),
    );

    const opening = useAgents.getState().openAgent("a");
    await useAgents.getState().openAgent("b");
    answerA(ok(view("a")));
    await opening;

    expect(useAgents.getState().open?.name).toBe("b");
    expect(useAgents.getState().draft?.body).toBe("b's body");
  });

  it("an agent read after the editor was closed is dropped", async () => {
    let answerA: (value: unknown) => void = () => {};
    agentGet.mockReturnValue(
      new Promise((resolve) => {
        answerA = resolve;
      }),
    );

    const opening = useAgents.getState().openAgent("a");
    useAgents.getState().close();
    answerA(ok(view("a")));
    await opening;

    expect(useAgents.getState().open).toBeNull();
    expect(useAgents.getState().draft).toBeNull();
  });
});
