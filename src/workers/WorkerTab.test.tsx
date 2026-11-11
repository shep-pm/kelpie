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
  NoteOutcome,
  SandboxAnswer,
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
const workerNote = vi.fn(
  async (_ticket: string, _text: string): Promise<Ok<NoteOutcome> | Err> =>
    ok("queued"),
);
const workerStopAndTell = vi.fn(
  async (_ticket: string, _text: string): Promise<Ok<NoteOutcome> | Err> =>
    ok("sent"),
);
const workerCancel = vi.fn(
  async (_ticket: string): Promise<Ok<null> | Err> => ok(null),
);
const workerAnswer = vi.fn(
  async (
    _ticket: string,
    _id: string,
    _answer: SandboxAnswer,
  ): Promise<Ok<null> | Err> => ok(null),
);
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
    workerNote: (ticket: string, text: string) => workerNote(ticket, text),
    workerStopAndTell: (ticket: string, text: string) =>
      workerStopAndTell(ticket, text),
    workerCancel: (ticket: string) => workerCancel(ticket),
    workerAnswer: (ticket: string, id: string, answer: SandboxAnswer) =>
      workerAnswer(ticket, id, answer),
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
  workerNote.mockClear();
  workerStopAndTell.mockClear();
  workerCancel.mockClear();
  workerAnswer.mockClear();
  for (const call of Object.values(pm)) call.mockReset();
  useBoard.getState().reset();
});
afterEach(cleanup);

async function open(entries: LogEntry[] = past) {
  watching(entries);
  render(<WorkerTab ticket={TICKET} />);
  await waitFor(() => expect(channels).toHaveLength(1));
}

/**
 * The last turn of the history was cut off, so the worker going on shows it was
 * only running: the tab is now looking at a turn in flight.
 */
async function runningTurn() {
  await act(async () =>
    channels[0]?.onmessage({ kind: "text", delta: " still at it" }),
  );
  return screen.findByPlaceholderText("Message the worker on #13");
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

  it("has a composer and no project picker", async () => {
    await open();
    await screen.findByText("Opening it now");

    expect(
      screen.getByPlaceholderText("Message the worker on #13"),
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Send" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Pick a folder" })).toBeNull();
  });

  it("shows Note, Stop and tell and Stop while a turn runs, and never Send", async () => {
    await open();
    await runningTurn();

    expect(await screen.findByRole("button", { name: "Note" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Stop and tell" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Stop" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Send" })).toBeNull();
  });

  it("shows Note alone between turns", async () => {
    await open();
    await screen.findByText("Opening it now");

    expect(await screen.findByRole("button", { name: "Note" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Stop and tell" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Send" })).toBeNull();
  });

  it("a Note while a turn runs calls workerNote and shows as queued under the turn", async () => {
    await open();
    const box = await runningTurn();

    fireEvent.change(box, { target: { value: "use the new name" } });
    fireEvent.click(screen.getByRole("button", { name: "Note" }));

    await waitFor(() =>
      expect(workerNote).toHaveBeenCalledWith("13", "use the new name"),
    );
    expect(await screen.findByText("use the new name")).toBeTruthy();
    expect(
      await screen.findByText("Queued, the worker reads it when its turn ends"),
    ).toBeTruthy();
  });

  it("a Note between turns that went out shows as a message of its own", async () => {
    workerNote.mockResolvedValueOnce(ok("sent"));
    await open(past.slice(0, 3));
    const box = await screen.findByPlaceholderText("Message the worker on #13");

    fireEvent.change(box, { target: { value: "carry on" } });
    fireEvent.click(screen.getByRole("button", { name: "Note" }));

    await waitFor(() =>
      expect(workerNote).toHaveBeenCalledWith("13", "carry on"),
    );
    expect(await screen.findByText("carry on")).toBeTruthy();
    expect(
      screen.queryByText("Queued, kelpie sends it when the worker next runs"),
    ).toBeNull();
    expect(screen.getAllByRole("article")).toHaveLength(2);
  });

  it("a Note between turns that the record kept says so, and is no turn", async () => {
    workerNote.mockResolvedValueOnce(ok("queued"));
    await open(past.slice(0, 3));
    const box = await screen.findByPlaceholderText("Message the worker on #13");

    fireEvent.change(box, { target: { value: "carry on" } });
    fireEvent.click(screen.getByRole("button", { name: "Note" }));

    expect(
      await screen.findByText(
        "Queued, kelpie sends it when the worker next runs",
      ),
    ).toBeTruthy();
    expect(await screen.findByText("carry on")).toBeTruthy();
    expect(
      screen.queryByText("Queued, the worker reads it when its turn ends"),
    ).toBeNull();
    expect(screen.getAllByRole("article")).toHaveLength(1);
    expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Stop and tell" })).toBeNull();
  });

  it("a Note the session sent as its next turn says so, and not that it waits", async () => {
    workerNote.mockResolvedValueOnce(ok("sent"));
    await open();
    const box = await runningTurn();

    fireEvent.change(box, { target: { value: "use the new name" } });
    fireEvent.click(screen.getByRole("button", { name: "Note" }));

    expect(
      await screen.findByText("Sent as the worker's next prompt"),
    ).toBeTruthy();
    expect(
      screen.queryByText("Queued, the worker reads it when its turn ends"),
    ).toBeNull();
  });

  it("a Stop and tell the session kept says so, and not that it went out", async () => {
    workerStopAndTell.mockResolvedValueOnce(ok("queued"));
    await open();
    const box = await runningTurn();

    fireEvent.change(box, { target: { value: "stop, fetch the log" } });
    fireEvent.click(screen.getByRole("button", { name: "Stop and tell" }));

    await waitFor(() =>
      expect(workerStopAndTell).toHaveBeenCalledWith(
        "13",
        "stop, fetch the log",
      ),
    );
    expect(
      await screen.findByText("Queued, the worker reads it when its turn ends"),
    ).toBeTruthy();
    expect(screen.queryByText("Sent as the worker's next prompt")).toBeNull();
  });

  it("Stop and tell calls workerStopAndTell with the composer's text, and Stop calls workerCancel", async () => {
    await open();
    const box = await runningTurn();

    fireEvent.change(box, { target: { value: "stop, fetch the log" } });
    fireEvent.click(screen.getByRole("button", { name: "Stop and tell" }));

    await waitFor(() =>
      expect(workerStopAndTell).toHaveBeenCalledWith(
        "13",
        "stop, fetch the log",
      ),
    );
    expect(
      await screen.findByText("Sent as the worker's next prompt"),
    ).toBeTruthy();
    expect(workerNote).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    await waitFor(() => expect(workerCancel).toHaveBeenCalledWith("13"));
  });

  it("Enter notes and Cmd+Enter stops and tells", async () => {
    await open();
    const box = await runningTurn();

    fireEvent.change(box, { target: { value: "one" } });
    fireEvent.keyDown(box, { key: "Enter" });
    await waitFor(() => expect(workerNote).toHaveBeenCalledWith("13", "one"));

    fireEvent.change(box, { target: { value: "two" } });
    fireEvent.keyDown(box, { key: "Enter", metaKey: true });
    await waitFor(() =>
      expect(workerStopAndTell).toHaveBeenCalledWith("13", "two"),
    );
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

  it("shows a sandbox request the worker asked for, named as the worker's", async () => {
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

    expect(await screen.findByText("Worker wants to reach a.dev")).toBeTruthy();
    expect(screen.getByText("docs")).toBeTruthy();
  });

  it("answers a sandbox request with workerAnswer", async () => {
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

    await waitFor(() =>
      expect(workerAnswer).toHaveBeenCalledWith("13", "net-1", "once"),
    );
    expect(
      await screen.findByText("a.dev allowed for this session"),
    ).toBeTruthy();
    expect(pm.pmAnswer).not.toHaveBeenCalled();
  });

  it("shows why an answer the worker's session refused did not land", async () => {
    workerAnswer.mockResolvedValueOnce({
      status: "error",
      error: "a.dev is not through a sandbox kelpie can widen",
    });
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

    expect(
      await screen.findByText(
        "a.dev is not through a sandbox kelpie can widen",
      ),
    ).toBeTruthy();
  });
});
