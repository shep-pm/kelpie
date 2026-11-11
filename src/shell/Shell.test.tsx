import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import {
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";

vi.mock("@/bindings", () => ({
  commands: { projectGet: async () => "/work/project" },
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
