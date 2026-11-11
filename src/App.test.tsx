import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";

vi.mock("@/bindings", () => ({
  commands: {
    projectGet: async () => "/work/project",
    settingsSchema: async () => ({ type: "object", properties: {} }),
    settingsGet: async () => ({
      status: "ok",
      data: { global: [], project: null },
    }),
    githubStatus: async () => ({ status: "ok", data: { kind: "no_gh" } }),
  },
}));

import { App } from "./App";

beforeAll(() => {
  // The resizable panels measure themselves, which jsdom cannot do.
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
});

afterEach(cleanup);

describe("App", () => {
  it("shows the PM's chat box", async () => {
    render(<App />);
    expect(await screen.findByPlaceholderText("Message the PM")).toBeTruthy();
  });

  it("opens the settings from the gear in the header and returns to the chat", async () => {
    render(<App />);
    await screen.findByPlaceholderText("Message the PM");

    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(await screen.findByRole("tab", { name: "Global" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Back to the chat" }));
    expect(screen.queryByRole("tab", { name: "Global" })).toBeNull();
    expect(screen.getByPlaceholderText("Message the PM")).toBeTruthy();
  });

  it("keeps a half-written message while the settings are open", async () => {
    render(<App />);
    const box = await screen.findByPlaceholderText("Message the PM");
    fireEvent.change(box, { target: { value: "half a thought" } });

    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    await screen.findByRole("tab", { name: "Global" });
    fireEvent.click(screen.getByRole("button", { name: "Back to the chat" }));

    expect(
      (screen.getByPlaceholderText("Message the PM") as HTMLTextAreaElement)
        .value,
    ).toBe("half a thought");
  });
});
