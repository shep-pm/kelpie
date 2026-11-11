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
    boardWatch: async (channel: { onmessage: (event: unknown) => void }) => {
      channel.onmessage({
        kind: "board",
        held_until: null,
        tickets: [
          {
            id: "12",
            title: "Wire the poller",
            priority: null,
            column: "ready",
            url: "https://github.com/o/r/issues/12",
            pull_url: null,
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
    fireEvent.click(screen.getByRole("button", { name: "Open Board" }));
    expect(board()).toBeTruthy();
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

  it("opens the settings from the gear in the PM's header", () => {
    const onSettings = vi.fn();
    render(<Shell onSettings={onSettings} />);
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(onSettings).toHaveBeenCalledOnce();
  });
});
