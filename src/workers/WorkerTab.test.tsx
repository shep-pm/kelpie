import type { Channel } from "@tauri-apps/api/core";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  BoardView,
  LogEntry,
  SessionEvent,
  WorkerStarted,
} from "@/bindings";

// The real Channel needs Tauri's IPC bridge, which jsdom has not got.
vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    onmessage: (event: unknown) => void = () => {};
  },
}));

type Ok<T> = { status: "ok"; data: T };
type Err = { status: "error"; error: string };
const ok = <T,>(data: T): Ok<T> => ({ status: "ok", data });

/** The channels `workerWatch` was given, oldest first. */
const channels: Channel<SessionEvent>[] = [];
const workerWatch = vi.fn(
  async (
    _ticket: string,
    channel: Channel<SessionEvent>,
  ): Promise<Ok<LogEntry[]> | Err> => {
    channels.push(channel);
    return ok([]);
  },
);
const workerUnwatch = vi.fn(
  async (_ticket: string, _channel: Channel<SessionEvent>) => ok(null),
);
const workerResume = vi.fn(
  async (ticket: string): Promise<Ok<WorkerStarted> | Err> =>
    ok({ ticket, waiting: false }),
);
const boardRefresh = vi.fn(async () => {});
const pm = {
  pmHistory: vi.fn(),
  pmSend: vi.fn(),
  pmRetry: vi.fn(),
  pmCancel: vi.fn(),
  pmAnswer: vi.fn(),
  pmSandbox: vi.fn(),
  sandboxInstall: vi.fn(),
  projectGet: vi.fn(),
};

vi.mock("@/bindings", () => ({
  commands: {
    workerWatch: (ticket: string, channel: Channel<SessionEvent>) =>
      workerWatch(ticket, channel),
    workerUnwatch: (ticket: string, channel: Channel<SessionEvent>) =>
      workerUnwatch(ticket, channel),
    workerResume: (ticket: string) => workerResume(ticket),
    boardRefresh: () => boardRefresh(),
    pmHistory: () => pm.pmHistory(),
    pmSend: () => pm.pmSend(),
    pmRetry: () => pm.pmRetry(),
    pmCancel: () => pm.pmCancel(),
    pmAnswer: () => pm.pmAnswer(),
    pmSandbox: () => pm.pmSandbox(),
    sandboxInstall: () => pm.sandboxInstall(),
    projectGet: () => pm.projectGet(),
  },
}));

import { useBoard } from "@/board/store";
import { WorkerTab } from "./WorkerTab";

const TICKET = { id: "13", title: "Open the draft" };

const past: LogEntry[] = [
  { entry: "user", text: "Open the draft pull request" },
  { entry: "event", kind: "text", delta: "I read the issue." },
  { entry: "event", kind: "turn_ended", stop_reason: "end_turn", usage: null },
  { entry: "user", text: "Carry on from where you stopped." },
  { entry: "event", kind: "text", delta: "Opening it now" },
];

function watching(entries: LogEntry[]) {
  workerWatch.mockImplementationOnce(async (_ticket, channel) => {
    channels.push(channel);
    return ok(entries);
  });
}

function onBoard(needsYou: string[]) {
  const view: BoardView = {
    held_until: null,
    needs_you: needsYou.length,
    tickets: [
      {
        id: "13",
        title: "Open the draft",
        priority: null,
        column: "building",
        url: "https://github.com/o/r/issues/13",
        pull_url: null,
        needs_you: needsYou,
      },
    ],
  };
  useBoard.setState({ view });
}

beforeEach(() => {
  channels.length = 0;
  workerWatch.mockClear();
  workerUnwatch.mockClear();
  workerResume.mockClear();
  boardRefresh.mockClear();
  for (const call of Object.values(pm)) call.mockReset();
  useBoard.getState().reset();
});
afterEach(cleanup);

async function open(entries: LogEntry[] = past) {
  watching(entries);
  render(<WorkerTab ticket={TICKET} />);
  await waitFor(() => expect(channels).toHaveLength(1));
}

describe("a worker's tab", () => {
  it("names the ticket in its header", async () => {
    await open();

    const header = screen.getByRole("heading");
    expect(header.textContent).toContain("#13");
    expect(header.textContent).toContain("Open the draft");
  });

  it("shows the worker's past turns from workerWatch's entries", async () => {
    await open();

    expect(await screen.findByText("Open the draft pull request")).toBeTruthy();
    expect(screen.getByText("I read the issue.")).toBeTruthy();
    expect(screen.getByText("Carry on from where you stopped.")).toBeTruthy();
    expect(screen.getByText("Opening it now")).toBeTruthy();
    expect(workerWatch).toHaveBeenCalledWith("13", channels[0]);
  });

  it("appends an event on the watch channel to the running turn", async () => {
    await open();
    await screen.findByText("Opening it now");

    await act(async () =>
      channels[0]?.onmessage({ kind: "text", delta: " and pushing" }),
    );

    expect(screen.getByText("Opening it now and pushing")).toBeTruthy();
  });

  it("shows the turn a Resume starts as a turn of its own, running", async () => {
    await open(past.slice(0, 3));
    await screen.findByText("I read the issue.");

    await act(async () =>
      channels[0]?.onmessage({ kind: "text", delta: "Back at it" }),
    );

    expect(screen.getByText("I read the issue.")).toBeTruthy();
    expect(screen.getByText("Back at it")).toBeTruthy();
    expect(screen.queryByText("I read the issueBack at it")).toBeNull();
    expect(screen.getAllByRole("article")).toHaveLength(2);
  });

  it("shows a worker that ended with its last turn as it is", async () => {
    await open(past.slice(0, 3));

    expect(await screen.findByText("I read the issue.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Resume" })).toBeNull();
  });

  it("has no composer and no project picker", async () => {
    await open();
    await screen.findByText("Opening it now");

    expect(screen.queryByLabelText("Message")).toBeNull();
    expect(screen.queryByRole("button", { name: "Send" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Pick a folder" })).toBeNull();
  });

  it("calls no pm_ command", async () => {
    await open();
    await screen.findByText("Opening it now");
    onBoard(["Paused, resumes when you say so"]);
    fireEvent.click(await screen.findByRole("button", { name: "Resume" }));
    await waitFor(() => expect(workerResume).toHaveBeenCalled());
    cleanup();

    for (const call of Object.values(pm)) expect(call).not.toHaveBeenCalled();
  });

  it("shows Resume for a Paused worker, which calls workerResume with its id", async () => {
    onBoard(["Paused, resumes when you say so"]);
    await open();

    fireEvent.click(await screen.findByRole("button", { name: "Resume" }));

    await waitFor(() => expect(workerResume).toHaveBeenCalledWith("13"));
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Resume" })).toBeNull(),
    );
  });

  it("shows Resume again when the worker is Paused again after a Resume", async () => {
    onBoard(["Paused, resumes when you say so"]);
    await open();
    fireEvent.click(await screen.findByRole("button", { name: "Resume" }));
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Resume" })).toBeNull(),
    );

    await act(async () => onBoard([]));
    await act(async () => onBoard(["Paused, resumes when you say so"]));

    expect(await screen.findByRole("button", { name: "Resume" })).toBeTruthy();
  });

  it("shows why a Resume was refused, and keeps the button", async () => {
    workerResume.mockResolvedValueOnce({
      status: "error",
      error: "ticket 13 is running, so it does not resume",
    });
    onBoard(["Paused, resumes when you say so"]);
    await open();

    fireEvent.click(await screen.findByRole("button", { name: "Resume" }));

    expect(
      await screen.findByText("ticket 13 is running, so it does not resume"),
    ).toBeTruthy();
    expect(screen.getByRole("button", { name: "Resume" })).toBeTruthy();
  });

  it("shows no Resume for a worker that is not Paused", async () => {
    onBoard(["Waiting for net access"]);
    await open();
    await screen.findByText("Opening it now");

    expect(screen.queryByRole("button", { name: "Resume" })).toBeNull();
  });

  it("stops watching when its tab closes", async () => {
    await open();
    await screen.findByText("Opening it now");

    cleanup();

    await waitFor(() =>
      expect(workerUnwatch).toHaveBeenCalledWith("13", channels[0]),
    );
  });

  it("waits for a watch still in flight before it unwatches", async () => {
    let answer: (entries: Ok<LogEntry[]>) => void = () => {};
    workerWatch.mockImplementationOnce(
      (_ticket, channel) =>
        new Promise((resolve) => {
          channels.push(channel);
          answer = resolve;
        }),
    );
    render(<WorkerTab ticket={TICKET} />);
    await waitFor(() => expect(channels).toHaveLength(1));

    cleanup();
    await act(async () => {});
    expect(workerUnwatch).not.toHaveBeenCalled();

    await act(async () => answer(ok([])));
    await waitFor(() =>
      expect(workerUnwatch).toHaveBeenCalledWith("13", channels[0]),
    );
  });

  it("answers a sandbox request with a line saying the worker's own wait for #153", async () => {
    await open([
      { entry: "user", text: "go" },
      {
        entry: "event",
        kind: "sandbox_request",
        id: "net-1",
        host: "a.dev",
        why: "docs",
      },
    ]);

    fireEvent.click(await screen.findByRole("button", { name: "Allow once" }));

    expect(await screen.findByText(/#153/)).toBeTruthy();
  });
});
