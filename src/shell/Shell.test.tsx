import {
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import {
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";

// The real Channel needs Tauri's IPC bridge, which jsdom has not got.
vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    onmessage: (event: unknown) => void = () => {};
  },
}));

vi.mock("@/bindings", () => ({
  commands: {
    projectGet: async () => "/work/project",
    workerWatch: async () => ({
      status: "ok",
      data: [
        { entry: "user", text: "Open the draft pull request" },
        { entry: "event", kind: "text", delta: "I read the issue." },
      ],
    }),
    workerUnwatch: async () => ({ status: "ok", data: null }),
    boardWatch: async (channel: { onmessage: (event: unknown) => void }) => {
      channel.onmessage({
        kind: "board",
        held_until: null,
        needs_you: 2,
        tickets: [
          {
            id: "12",
            title: "Wire the poller",
            priority: null,
            column: "ready",
            url: "https://github.com/o/r/issues/12",
            pull_url: null,
            needs_you: [],
          },
          {
            id: "13",
            title: "Open the draft",
            priority: null,
            column: "building",
            url: "https://github.com/o/r/issues/13",
            pull_url: null,
            needs_you: [],
          },
          {
            id: "17",
            title: "Answer the refusal",
            priority: null,
            column: "building",
            url: "https://github.com/o/r/issues/17",
            pull_url: null,
            needs_you: [
              "GitHub refused the push",
              "Paused, resumes when you say so",
            ],
          },
        ],
      });
      return { status: "ok", data: null };
    },
  },
}));

import { Shell } from "./Shell";
import { useTabs } from "./tabs";

beforeAll(() => {
  // The resizable group measures itself, which jsdom cannot do.
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
});

beforeEach(() => useTabs.getState().reset());
afterEach(cleanup);

const pm = () => screen.queryByRole("region", { name: "PM" });
const board = () => screen.queryByRole("region", { name: "Board" });

describe("the shell", () => {
  it("shows a PM panel and a Board panel, and the PM's chat box", async () => {
    render(<Shell onSettings={() => {}} />);
    expect(pm()).toBeTruthy();
    expect(board()).toBeTruthy();
    expect(await screen.findByPlaceholderText("Message the PM")).toBeTruthy();
  });

  it("the Board panel shows the board", async () => {
    render(<Shell onSettings={() => {}} />);

    const ready = await screen.findByRole("group", { name: "Ready" });
    expect(within(ready).getByText("Wire the poller")).toBeTruthy();
    expect(
      within(screen.getByRole("region", { name: "Board" })).getByRole("group", {
        name: "Draft",
      }),
    ).toBeTruthy();
  });

  it("collapsing a panel hides its body and shows an icon button that reopens it", async () => {
    render(<Shell onSettings={() => {}} />);
    await screen.findByPlaceholderText("Message the PM");

    fireEvent.click(screen.getByRole("button", { name: "Collapse PM" }));
    expect(pm()).toBeNull();
    expect(screen.queryByPlaceholderText("Message the PM")).toBeNull();
    expect(board()).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Open PM" }));
    expect(pm()).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Open PM" })).toBeNull();
    expect(await screen.findByPlaceholderText("Message the PM")).toBeTruthy();
  });

  it("collapses the Board panel the same way", () => {
    render(<Shell onSettings={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Collapse Board" }));
    expect(board()).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /^Open Board/ }));
    expect(board()).toBeTruthy();
  });

  it("a collapsed Board panel's icon button carries the board's count", async () => {
    render(<Shell onSettings={() => {}} />);
    await screen.findByRole("group", { name: "Ready" });

    fireEvent.click(screen.getByRole("button", { name: "Collapse Board" }));

    const open = screen.getByRole("button", { name: /^Open Board/ });
    expect(open.textContent).toContain("2");
    expect(open.getAttribute("aria-label")).toBe("Open Board, 2 need you");
  });

  it("maximising a panel hides the other, and a second press restores it", () => {
    render(<Shell onSettings={() => {}} />);

    fireEvent.click(screen.getByRole("button", { name: "Maximise Board" }));
    expect(board()).toBeTruthy();
    expect(pm()).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Restore Board" }));
    expect(pm()).toBeTruthy();
    expect(board()).toBeTruthy();
  });

  it("clicking a Building ticket opens its worker's tab, closing it hides it, and clicking again reopens it with its history", async () => {
    render(<Shell onSettings={() => {}} />);

    const building = await screen.findByRole("group", { name: "Building" });
    fireEvent.click(within(building).getByText("Open the draft"));

    const tab = await screen.findByRole("tab", { name: "#13" });
    expect(tab.getAttribute("aria-selected")).toBe("true");
    expect(await screen.findByText("I read the issue.")).toBeTruthy();
    // The PM's own composer stays mounted behind the tab, hidden.
    const panel = screen.getByText("I read the issue.").closest("main");
    expect(panel && within(panel).queryByLabelText("Message")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Close #13" }));
    expect(screen.queryByRole("tab", { name: "#13" })).toBeNull();
    expect(screen.queryByText("I read the issue.")).toBeNull();

    fireEvent.click(within(building).getByText("Open the draft"));
    expect(await screen.findByRole("tab", { name: "#13" })).toBeTruthy();
    expect(await screen.findByText("I read the issue.")).toBeTruthy();
  });

  it("a worker's tab counts its own waits, and no other worker's", async () => {
    render(<Shell onSettings={() => {}} />);
    const building = await screen.findByRole("group", { name: "Building" });

    fireEvent.click(within(building).getByText("Open the draft"));
    fireEvent.click(within(building).getByText("Answer the refusal"));

    const waiting = await screen.findByRole("tab", { name: "#17, 2 need you" });
    expect(within(waiting).getByText("2")).toBeTruthy();
    expect(screen.getByRole("tab", { name: "#13" }).textContent).toBe("#13");
  });

  it("opens the settings from the gear in the PM's header", () => {
    const onSettings = vi.fn();
    render(<Shell onSettings={onSettings} />);
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(onSettings).toHaveBeenCalledOnce();
  });
});
