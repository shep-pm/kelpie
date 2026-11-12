import type { Channel } from "@tauri-apps/api/core";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { LogEntry, SessionEvent } from "@/bindings";

// The real Channel needs Tauri's IPC bridge, which jsdom has not got.
vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    onmessage: (event: unknown) => void = () => {};
  },
}));

type Ok<T> = { status: "ok"; data: T };
const ok = <T,>(data: T): Ok<T> => ({ status: "ok", data });

/** The channels `reviewWatch` was given, oldest first. */
const channels: Channel<SessionEvent>[] = [];
const reviewWatch = vi.fn(
  async (
    _ticket: string,
    channel: Channel<SessionEvent>,
  ): Promise<Ok<LogEntry[]> | { status: "error"; error: string }> => {
    channels.push(channel);
    return ok([]);
  },
);
const reviewUnwatch = vi.fn(
  async (_ticket: string, _channel: Channel<SessionEvent>) => ok(null),
);
/** Every other session the window could have been watching instead. */
const told = {
  workerWatch: vi.fn(),
  workerNote: vi.fn(),
  workerCancel: vi.fn(),
  pmSend: vi.fn(),
  pmHistory: vi.fn(),
  pmCancel: vi.fn(),
};

vi.mock("@/bindings", () => ({
  commands: {
    reviewWatch: (ticket: string, channel: Channel<SessionEvent>) =>
      reviewWatch(ticket, channel),
    reviewUnwatch: (ticket: string, channel: Channel<SessionEvent>) =>
      reviewUnwatch(ticket, channel),
    workerWatch: (ticket: string, channel: Channel<SessionEvent>) =>
      told.workerWatch(ticket, channel),
    workerNote: (ticket: string, text: string) => told.workerNote(ticket, text),
    workerCancel: (ticket: string) => told.workerCancel(ticket),
    pmSend: () => told.pmSend(),
    pmHistory: () => told.pmHistory(),
    pmCancel: () => told.pmCancel(),
  },
}));

import { ReviewerTab, type ReviewTicket } from "./ReviewerTab";

const TICKET: ReviewTicket = {
  id: "14",
  title: "Show the read as a tab",
  reviewer: "kelpie:reviewer",
};

/** One read that ran to its end: what kelpie told it, its tool calls and its reply. */
const first: LogEntry[] = [
  {
    entry: "user",
    text: "Read the pull request of #14 and list what it changes.",
  },
  { entry: "event", kind: "started", session_id: "s-1" },
  {
    entry: "event",
    kind: "tool_step",
    id: "t-1",
    title: "Read src-tauri/src/review/watch.rs",
    status: "done",
  },
  { entry: "event", kind: "text", delta: "The head adds a feed per ticket." },
  { entry: "event", kind: "turn_ended", stop_reason: "end_turn", usage: null },
];

/** The second look, which the feed put behind the line naming it. */
const second: LogEntry[] = [
  { entry: "divider", text: "kelpie:reviewer reads again" },
  { entry: "user", text: "Your first look found one finding. Read it again." },
  { entry: "event", kind: "text", delta: "The second look agrees." },
  { entry: "event", kind: "turn_ended", stop_reason: "end_turn", usage: null },
];

function watching(entries: LogEntry[]) {
  reviewWatch.mockImplementationOnce(async (_ticket, channel) => {
    channels.push(channel);
    return ok(entries);
  });
}

beforeEach(() => {
  channels.length = 0;
  reviewWatch.mockClear();
  reviewUnwatch.mockClear();
  for (const call of Object.values(told)) call.mockReset();
});
afterEach(cleanup);

async function open(
  entries: LogEntry[] = first,
  ticket: ReviewTicket = TICKET,
) {
  watching(entries);
  render(<ReviewerTab ticket={ticket} />);
  await waitFor(() => expect(channels).toHaveLength(1));
}

/** `a` shows before `b` in the tab. */
function showsBefore(a: HTMLElement, b: HTMLElement) {
  const following =
    a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING;
  expect(following).toBeTruthy();
}

describe("a reviewer's tab", () => {
  it("names the ticket and its reviewer in its header", async () => {
    await open();

    const heading = await screen.findByRole("heading");
    expect(heading.textContent).toContain("#14");
    expect(heading.textContent).toContain("Show the read as a tab");
    expect(screen.getByText("kelpie:reviewer")).toBeTruthy();
  });

  it("shows the read's prompt, its tool calls and its reply from reviewWatch's entries", async () => {
    await open();

    expect(
      await screen.findByText(
        "Read the pull request of #14 and list what it changes.",
      ),
    ).toBeTruthy();
    expect(screen.getByText("Read src-tauri/src/review/watch.rs")).toBeTruthy();
    expect(screen.getByText("The head adds a feed per ticket.")).toBeTruthy();
    expect(reviewWatch).toHaveBeenCalledWith("14", channels[0]);
  });

  it("has no prompt input, and no button that tells a session", async () => {
    await open();
    await screen.findByText("The head adds a feed per ticket.");

    expect(screen.queryByRole("textbox")).toBeNull();
    expect(screen.queryByPlaceholderText(/^Message/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Send" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Note" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Stop and tell" })).toBeNull();
  });

  it("appends an event on the watch channel to the read in flight", async () => {
    await open(first.slice(0, 4));
    await screen.findByText("The head adds a feed per ticket.");

    await act(async () =>
      channels[0]?.onmessage({ kind: "text", delta: " and no worker turn" }),
    );

    expect(
      screen.getByText("The head adds a feed per ticket. and no worker turn"),
    ).toBeTruthy();
  });

  it("shows no Retry for a read that was cut off, since the pass runs it again", async () => {
    await open(first.slice(0, 4));
    await screen.findByText("The head adds a feed per ticket.");

    expect(
      await screen.findByText("Interrupted when kelpie quit"),
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
  });

  it("shows a second look below the divider naming it", async () => {
    await open([...first, ...second]);

    const divider = await screen.findByText("kelpie:reviewer reads again");
    const again = screen.getByText(
      "Your first look found one finding. Read it again.",
    );
    const found = screen.getByText("The second look agrees.");

    showsBefore(divider, again);
    showsBefore(again, found);
    expect(screen.getAllByRole("article")).toHaveLength(2);
  });

  it("keeps the first look above the divider", async () => {
    await open([...first, ...second]);

    const reply = await screen.findByText("The head adds a feed per ticket.");
    const divider = screen.getByText("kelpie:reviewer reads again");

    showsBefore(reply, divider);
  });

  it("stops watching when its tab closes, and the read goes on", async () => {
    await open();
    await screen.findByText("The head adds a feed per ticket.");

    cleanup();

    await waitFor(() =>
      expect(reviewUnwatch).toHaveBeenCalledWith("14", channels[0]),
    );
  });

  it("waits for a watch still in flight before it unwatches", async () => {
    let answer: (entries: Ok<LogEntry[]>) => void = () => {};
    reviewWatch.mockImplementationOnce(
      (_ticket, channel) =>
        new Promise((resolve) => {
          channels.push(channel);
          answer = resolve;
        }),
    );
    render(<ReviewerTab ticket={TICKET} />);
    await waitFor(() => expect(channels).toHaveLength(1));

    cleanup();
    await act(async () => {});
    expect(reviewUnwatch).not.toHaveBeenCalled();

    await act(async () => answer(ok([])));
    await waitFor(() =>
      expect(reviewUnwatch).toHaveBeenCalledWith("14", channels[0]),
    );
  });

  it("says nothing to a read: no worker or PM command runs", async () => {
    await open([...first, ...second]);
    await screen.findByText("The second look agrees.");

    for (const call of Object.values(told)) expect(call).not.toHaveBeenCalled();
  });

  it("titles its header Reviewer when kelpie has seen no read of the ticket", async () => {
    await open([], { ...TICKET, reviewer: null });

    expect(await screen.findByText("Reviewer")).toBeTruthy();
    expect(screen.queryByText("kelpie:reviewer")).toBeNull();
  });
});
