import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/bindings", () => ({
  commands: { projectGet: async () => "/work/project" },
}));

import { PmPanel } from "./PmPanel";
import { registerTabBody, useTabs } from "./tabs";

function CountingBody() {
  const [n, setN] = useState(0);
  return (
    <button type="button" onClick={() => setN(n + 1)}>
      clicked {n}
    </button>
  );
}

const controls = {
  maximised: false,
  onCollapse: () => {},
  onMaximise: () => {},
};

beforeEach(() => useTabs.getState().reset());
afterEach(cleanup);

describe("the PM panel", () => {
  it("shows the PM tab with the chat box and no close button", async () => {
    render(<PmPanel {...controls} onSettings={() => {}} />);
    expect(await screen.findByPlaceholderText("Message the PM")).toBeTruthy();
    expect(screen.getByRole("tab", { name: "PM" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Close PM" })).toBeNull();
  });

  it("opens the settings from the gear in its header", () => {
    const onSettings = vi.fn();
    render(<PmPanel {...controls} onSettings={onSettings} />);
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(onSettings).toHaveBeenCalledOnce();
  });

  it("shows a tab the store opens in the strip with a close button, and its body once selected", async () => {
    registerTabBody("w1", () => <p>Worker 120's chat</p>);
    render(<PmPanel {...controls} onSettings={() => {}} />);
    await screen.findByPlaceholderText("Message the PM");

    act(() => useTabs.getState().open({ id: "w1", title: "#120" }));
    expect(screen.getByRole("tab", { name: "#120" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Close #120" })).toBeTruthy();
    expect(screen.getByText("Worker 120's chat")).toBeTruthy();

    fireEvent.mouseDown(screen.getByRole("tab", { name: "PM" }), { button: 0 });
    expect(screen.queryByText("Worker 120's chat")).toBeNull();

    fireEvent.mouseDown(screen.getByRole("tab", { name: "#120" }), {
      button: 0,
    });
    expect(screen.getByText("Worker 120's chat")).toBeTruthy();
  });

  it("closes a tab from its close button and returns to the PM", async () => {
    render(<PmPanel {...controls} onSettings={() => {}} />);
    act(() => useTabs.getState().open({ id: "w2", title: "#121" }));

    fireEvent.click(screen.getByRole("button", { name: "Close #121" }));
    expect(screen.queryByRole("tab", { name: "#121" })).toBeNull();
    expect(
      screen.getByRole("tab", { name: "PM", selected: true }),
    ).toBeTruthy();
  });

  it("keeps a half-written message while a worker's tab is shown", async () => {
    render(<PmPanel {...controls} onSettings={() => {}} />);
    const box = await screen.findByPlaceholderText("Message the PM");
    fireEvent.change(box, { target: { value: "half a thought" } });

    act(() => useTabs.getState().open({ id: "w3", title: "#122" }));
    act(() => useTabs.getState().select("pm"));

    expect(
      (screen.getByPlaceholderText("Message the PM") as HTMLTextAreaElement)
        .value,
    ).toBe("half a thought");
  });

  it("keeps working when a body that uses hooks is open beside more tabs", async () => {
    const unregister = registerTabBody("w4", CountingBody);
    render(<PmPanel {...controls} onSettings={() => {}} />);
    act(() => useTabs.getState().open({ id: "w4", title: "#123" }));
    fireEvent.click(screen.getByRole("button", { name: "clicked 0" }));
    expect(screen.getByRole("button", { name: "clicked 1" })).toBeTruthy();

    act(() => useTabs.getState().open({ id: "w5", title: "#124" }));
    act(() => useTabs.getState().close("w5"));
    act(() => useTabs.getState().select("w4"));
    // A tab that is not shown is unmounted, so its own state starts again.
    expect(screen.getByRole("button", { name: "clicked 0" })).toBeTruthy();
    unregister();
  });

  it("shows a body registered after its tab opened, and drops it when taken back", async () => {
    render(<PmPanel {...controls} onSettings={() => {}} />);
    act(() => useTabs.getState().open({ id: "w6", title: "#125" }));
    expect(screen.queryByText("Late body")).toBeNull();

    let unregister = () => {};
    act(() => {
      unregister = registerTabBody("w6", () => <p>Late body</p>);
    });
    expect(screen.getByText("Late body")).toBeTruthy();

    act(() => unregister());
    expect(screen.queryByText("Late body")).toBeNull();
  });
});
