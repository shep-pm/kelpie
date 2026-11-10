import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentSummary } from "@/bindings";

const ok = <T,>(data: T) => ({ status: "ok" as const, data });

const agentsList = vi.fn<() => Promise<unknown>>();
const agentNew =
  vi.fn<(name: string, parent: string | null) => Promise<unknown>>();

vi.mock("@/bindings", () => ({
  commands: {
    agentsList: () => agentsList(),
    agentNew: (name: string, parent: string | null) => agentNew(name, parent),
  },
}));
// The editor has its own tests
vi.mock("./AgentEditor", () => ({
  AgentEditor: ({ name }: { name: string }) => <p>editing {name}</p>,
}));

import { AgentsTab } from "./AgentsTab";
import { useAgents } from "./store";

const pm: AgentSummary = { name: "pm", exists: false, problem: null };
const odd: AgentSummary = {
  name: "odd",
  exists: true,
  problem: "unknown key `colour`",
};

beforeEach(() => {
  useAgents.getState().reset();
  agentsList.mockResolvedValue(ok([odd, pm]));
  agentNew.mockResolvedValue(ok(null));
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("the agents tab", () => {
  it("lists the agents, the global pm before it is saved, and why one cannot be used", async () => {
    render(<AgentsTab />);

    expect(await screen.findByRole("button", { name: /^pm/ })).toBeTruthy();
    expect(screen.getByText("builds on kelpie:pm until saved")).toBeTruthy();
    expect(screen.getByText("unknown key `colour`")).toBeTruthy();
  });

  it("opens an agent in the editor", async () => {
    render(<AgentsTab />);

    fireEvent.click(await screen.findByRole("button", { name: /^pm/ }));

    expect(screen.getByText("editing pm")).toBeTruthy();
  });

  it("makes a new agent that builds on a parent, and opens it", async () => {
    render(<AgentsTab />);
    await screen.findByRole("button", { name: /^pm/ });

    fireEvent.change(screen.getByLabelText("Name"), {
      target: { value: "reviewer" },
    });
    fireEvent.click(screen.getByRole("button", { name: "New agent" }));

    await waitFor(() =>
      expect(screen.getByText("editing reviewer")).toBeTruthy(),
    );
    expect(agentNew).toHaveBeenCalledWith("reviewer", "kelpie:pm");
  });

  it("shows why a new agent was refused and stays on the list", async () => {
    agentNew.mockResolvedValue({ status: "error", error: "agent pm exists" });
    render(<AgentsTab />);
    await screen.findByRole("button", { name: /^pm/ });

    fireEvent.change(screen.getByLabelText("Name"), {
      target: { value: "pm" },
    });
    fireEvent.click(screen.getByRole("button", { name: "New agent" }));

    expect((await screen.findByRole("alert")).textContent).toBe(
      "agent pm exists",
    );
    expect(screen.queryByText("editing pm")).toBeNull();
  });
});
