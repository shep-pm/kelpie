import type { Channel } from "@tauri-apps/api/core";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  InstallOutcome,
  LogEntry,
  ResolvedView,
  SessionEvent,
  Target,
  Value,
} from "@/bindings";

// The real Channel needs Tauri's IPC bridge, which jsdom has not got.
vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    onmessage: (event: unknown) => void = () => {};
  },
}));

type Done<T> = { status: "ok"; data: T } | { status: "error"; error: string };

const settingsGet =
  vi.fn<(project: string | null) => Promise<Done<ResolvedView>>>();
const settingsSet =
  vi.fn<(layer: Target, path: string, value: Value) => Promise<Done<null>>>();
const pmSend =
  vi.fn<(text: string, on: Channel<SessionEvent>) => Promise<undefined>>();
const pmHistory = vi.fn<() => Promise<Done<LogEntry[]>>>();
const pmSandbox = vi.fn();
const sandboxInstall = vi.fn<() => Promise<InstallOutcome>>();

vi.mock("@/bindings", () => ({
  commands: {
    settingsGet: (project: string | null) => settingsGet(project),
    settingsSet: (layer: Target, path: string, value: Value) =>
      settingsSet(layer, path, value),
    pmSend: (text: string, on: Channel<SessionEvent>) => pmSend(text, on),
    pmHistory: () => pmHistory(),
    pmSandbox: () => pmSandbox(),
    sandboxInstall: () => sandboxInstall(),
  },
}));

import { SandboxSetup } from "./SandboxSetup";
import { useChat } from "./store";

const view: ResolvedView = {
  global: [],
  project: { id: "p-1", path: "/work/project", fields: [] },
};

const BUTTON = { name: "Run without a sandbox" };
const INSTALL = { name: "Install" };

/** The chat as it stands when the sandbox refused the first message. */
function refused() {
  useChat.setState({
    turns: [
      {
        id: 1,
        dividers: [],
        user: "plan the slice",
        reply: "",
        steps: [],
        sandboxRequests: [],
        notes: [],
        usage: null,
        state: "failed",
        error: "bwrap: loopback: Failed RTM_NEWADDR",
      },
    ],
  });
}

describe("the sandbox setup item", () => {
  beforeEach(() => {
    settingsGet.mockReset();
    settingsGet.mockResolvedValue({ status: "ok", data: view });
    settingsSet.mockReset();
    settingsSet.mockResolvedValue({ status: "ok", data: null });
    pmSend.mockReset();
    pmSend.mockResolvedValue(undefined);
    pmHistory.mockReset();
    pmHistory.mockResolvedValue({ status: "ok", data: [] });
    pmSandbox.mockReset();
    pmSandbox.mockResolvedValue({
      refusal: null,
      unsandboxed: false,
      install: "available",
    });
    sandboxInstall.mockReset();
    sandboxInstall.mockResolvedValue({ kind: "installed" });
    useChat.getState().reset();
  });
  afterEach(cleanup);

  it("no_capabilities says Ubuntu restricts the sandbox", () => {
    render(<SandboxSetup refusal="no_capabilities" />);

    expect(screen.getByText("Ubuntu restricts the sandbox")).toBeTruthy();
    expect(
      screen.getByText(
        "Ubuntu 24.04 and later stop the sandbox from starting until kelpie's AppArmor profile is installed",
      ),
    ).toBeTruthy();
  });

  it("no_namespaces says this kernel has no sandbox", () => {
    render(<SandboxSetup refusal="no_namespaces" />);

    expect(screen.getByText("This kernel has no sandbox")).toBeTruthy();
    expect(
      screen.getByText(
        "The kernel gives no program a user namespace, and no profile can change that",
      ),
    ).toBeTruthy();
  });

  it.each(["no_capabilities", "no_namespaces"] as const)(
    "offers Run without a sandbox on %s",
    (refusal) => {
      render(<SandboxSetup refusal={refusal} />);

      expect(screen.getByRole("button", BUTTON)).toBeTruthy();
    },
  );

  it("a click turns the project's setting on and sends the failed message again", async () => {
    refused();
    render(<SandboxSetup refusal="no_capabilities" />);

    fireEvent.click(screen.getByRole("button", BUTTON));

    await waitFor(() => expect(pmSend).toHaveBeenCalledTimes(1));
    expect(settingsGet).toHaveBeenCalledWith(null);
    expect(settingsSet).toHaveBeenCalledWith(
      { kind: "project", id: "p-1" },
      "run-without-sandbox",
      true,
    );
    expect(pmSend).toHaveBeenCalledWith("plan the slice", expect.anything());
    // The write landed before the message went
    expect(settingsSet.mock.invocationCallOrder[0]).toBeLessThan(
      pmSend.mock.invocationCallOrder[0] ?? 0,
    );
  });

  it("a failed write shows its error in the item and sends nothing", async () => {
    refused();
    settingsSet.mockResolvedValue({ status: "error", error: "disk is full" });
    render(<SandboxSetup refusal="no_capabilities" />);

    fireEvent.click(screen.getByRole("button", BUTTON));

    const line = await screen.findByText("disk is full");
    expect(line.getAttribute("role")).toBe("alert");
    expect(pmSend).not.toHaveBeenCalled();
    // The button is still there to try again
    expect(screen.getByRole("button", BUTTON)).toBeTruthy();
  });

  it("with no project to write to it says so and sends nothing", async () => {
    refused();
    settingsGet.mockResolvedValue({
      status: "ok",
      data: { global: [], project: null },
    });
    render(<SandboxSetup refusal="no_capabilities" />);

    fireEvent.click(screen.getByRole("button", BUTTON));

    expect(await screen.findByText(/no project/i)).toBeTruthy();
    expect(settingsSet).not.toHaveBeenCalled();
    expect(pmSend).not.toHaveBeenCalled();
  });

  it("a read of the settings that fails shows its error and sends nothing", async () => {
    refused();
    settingsGet.mockResolvedValue({ status: "error", error: "no settings" });
    render(<SandboxSetup refusal="no_capabilities" />);

    fireEvent.click(screen.getByRole("button", BUTTON));

    expect(await screen.findByText("no settings")).toBeTruthy();
    expect(pmSend).not.toHaveBeenCalled();
  });

  it("stays while the PM runs unsandboxed, says so and has no button", () => {
    render(<SandboxSetup refusal="no_capabilities" unsandboxed />);

    expect(screen.getByText("Ubuntu restricts the sandbox")).toBeTruthy();
    expect(screen.getByText(/runs without a sandbox/i)).toBeTruthy();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("Install shows on available only", () => {
    const { unmount } = render(
      <SandboxSetup refusal="no_capabilities" install="available" />,
    );
    expect(screen.getByRole("button", INSTALL)).toBeTruthy();
    expect(screen.getByRole("button", BUTTON)).toBeTruthy();
    expect(screen.queryByText(/\.deb or \.rpm/)).toBeNull();
    unmount();

    // An AppImage or a dev build: Run without a sandbox stays, Install goes
    render(<SandboxSetup refusal="no_capabilities" install="package_only" />);
    expect(screen.queryByRole("button", INSTALL)).toBeNull();
    expect(screen.getByRole("button", BUTTON)).toBeTruthy();
    expect(
      screen.getByText("The .deb or .rpm can install the profile."),
    ).toBeTruthy();
  });

  it("a kernel with no namespaces is offered no Install, which no profile fixes", () => {
    render(<SandboxSetup refusal="no_namespaces" install="available" />);

    expect(screen.queryByRole("button", INSTALL)).toBeNull();
    expect(screen.queryByText(/\.deb or \.rpm/)).toBeNull();
  });

  it("Install is disabled while a turn runs", () => {
    useChat.setState({
      turns: [
        {
          id: 1,
          dividers: [],
          user: "go",
          reply: "",
          steps: [],
          sandboxRequests: [],
          notes: [],
          usage: null,
          state: "running",
        },
      ],
    });
    render(
      <SandboxSetup
        refusal="no_capabilities"
        unsandboxed
        install="available"
      />,
    );

    const button = screen.getByRole("button", INSTALL) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    fireEvent.click(button);
    expect(sandboxInstall).not.toHaveBeenCalled();
  });

  it("an unsandboxed PM keeps its item with Install and without Run without a sandbox", () => {
    render(
      <SandboxSetup
        refusal="no_capabilities"
        unsandboxed
        install="available"
      />,
    );

    expect(screen.getByText(/runs without a sandbox/i)).toBeTruthy();
    expect(screen.getByRole("button", INSTALL)).toBeTruthy();
    expect(screen.queryByRole("button", BUTTON)).toBeNull();
  });

  it("a finished install sends the failed message again", async () => {
    refused();
    render(<SandboxSetup refusal="no_capabilities" install="available" />);

    fireEvent.click(screen.getByRole("button", INSTALL));

    await waitFor(() => expect(pmSend).toHaveBeenCalledTimes(1));
    expect(sandboxInstall).toHaveBeenCalledTimes(1);
    expect(pmSend).toHaveBeenCalledWith("plan the slice", expect.anything());
    // Nothing was written to the project's settings
    expect(settingsSet).not.toHaveBeenCalled();
  });

  it("no agent shows the two commands to run in a terminal", async () => {
    refused();
    sandboxInstall.mockResolvedValue({
      kind: "no_agent",
      commands: [
        "sudo install -m 0644 /usr/share/kelpie-app/kelpie.apparmor /etc/apparmor.d/kelpie",
        "sudo apparmor_parser -r /etc/apparmor.d/kelpie",
      ],
    });
    render(<SandboxSetup refusal="no_capabilities" install="available" />);

    fireEvent.click(screen.getByRole("button", INSTALL));

    expect(
      await screen.findByText(
        "sudo install -m 0644 /usr/share/kelpie-app/kelpie.apparmor /etc/apparmor.d/kelpie",
      ),
    ).toBeTruthy();
    expect(
      screen.getByText("sudo apparmor_parser -r /etc/apparmor.d/kelpie"),
    ).toBeTruthy();
    expect(pmSend).not.toHaveBeenCalled();
  });

  it("a failure shows the tool's error as an alert and sends nothing", async () => {
    refused();
    sandboxInstall.mockResolvedValue({
      kind: "failed",
      message: "apparmor_parser: Unable to replace kelpie",
    });
    render(<SandboxSetup refusal="no_capabilities" install="available" />);

    fireEvent.click(screen.getByRole("button", INSTALL));

    const line = await screen.findByText(
      "apparmor_parser: Unable to replace kelpie",
    );
    expect(line.getAttribute("role")).toBe("alert");
    expect(pmSend).not.toHaveBeenCalled();
    // The button is still there to try again
    expect(
      (screen.getByRole("button", INSTALL) as HTMLButtonElement).disabled,
    ).toBe(false);
  });

  it("a dismissed prompt is a muted line, not an alert", async () => {
    refused();
    sandboxInstall.mockResolvedValue({ kind: "dismissed" });
    render(<SandboxSetup refusal="no_capabilities" install="available" />);

    fireEvent.click(screen.getByRole("button", INSTALL));

    const line = await screen.findByText(/prompt was dismissed/i);
    expect(line.getAttribute("role")).toBeNull();
    expect(line.className).toContain("text-muted-foreground");
    expect(pmSend).not.toHaveBeenCalled();
  });
});
