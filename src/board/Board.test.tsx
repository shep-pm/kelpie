import type { Channel } from "@tauri-apps/api/core";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { StrictMode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  BoardEvent,
  BoardView,
  Column,
  LinkView,
  MoveView,
  TicketView,
  WorkerStarted,
} from "@/bindings";
import {
  cardsIn,
  dragTo as dragCardTo,
  settle,
  stubBoardDragDom,
} from "@/test/board-drag";

// The real Channel needs Tauri's IPC bridge, which jsdom has not got.
vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    onmessage: (event: unknown) => void = () => {};
  },
}));

type Ok<T> = { status: "ok"; data: T };
type Err = { status: "error"; error: string };
const ok = <T,>(data: T): Ok<T> => ({ status: "ok", data });
const err = (error: string): Err => ({ status: "error", error });

/** The channels `boardWatch` was given, oldest first. */
const channels: Channel<BoardEvent>[] = [];
const boardWatch = vi.fn(
  async (channel: Channel<BoardEvent>): Promise<Ok<null> | Err> => {
    channels.push(channel);
    return ok(null);
  },
);
const boardRefresh = vi.fn(async () => {});
const boardMove = vi.fn(
  async (_ticket: string, _to: Column): Promise<Ok<MoveView> | Err> =>
    ok({ kind: "stay" }),
);
const workerStart = vi.fn(
  async (ticket: string): Promise<Ok<WorkerStarted> | Err> =>
    ok({ ticket, waiting: false }),
);
const openLink = vi.fn(
  async (_url: string): Promise<Ok<LinkView> | Err> => ok({ kind: "opened" }),
);

vi.mock("@/bindings", () => ({
  commands: {
    boardWatch: (channel: Channel<BoardEvent>) => boardWatch(channel),
    boardRefresh: () => boardRefresh(),
    boardMove: (ticket: string, to: Column) => boardMove(ticket, to),
    workerStart: (ticket: string) => workerStart(ticket),
    openLink: (url: string) => openLink(url),
  },
}));

import { Board } from "./Board";
import { useBoard } from "./store";

const ticket = (
  id: string,
  title: string,
  column: Column,
  more: Partial<TicketView> = {},
): TicketView => ({
  id,
  title,
  priority: null,
  column,
  url: `https://github.com/o/r/issues/${id}`,
  pull_url: null,
  needs_you: [],
  ...more,
});

const VIEW: BoardView = {
  held_until: null,
  needs_you: 0,
  tickets: [
    ticket("11", "Sketch the graph", "draft"),
    ticket("12", "Wire the poller", "ready", { priority: 1 }),
    ticket("13", "Open the draft", "building", { priority: 0 }),
    ticket("14", "Read the board", "review", {
      pull_url: "https://github.com/o/r/pull/40",
    }),
    ticket("15", "Lift the board", "done"),
  ],
};

/** What GitHub and the engine send the window, as the watching channel's handler gets it. */
async function emit(event: BoardEvent) {
  const channel = channels.at(-1);
  if (!channel) throw new Error("the board was never watched");
  await act(async () => channel.onmessage(event));
}
const emitBoard = (view: BoardView = VIEW) => emit({ kind: "board", ...view });

async function open(view: BoardView = VIEW, onOpenWorker = vi.fn()) {
  render(<Board onOpenWorker={onOpenWorker} />);
  await waitFor(() => expect(channels).toHaveLength(1));
  await emitBoard(view);
  return { onOpenWorker };
}

/**
 * Wait for the board's status line to say `text`. The line is always on the
 * page, empty until a notice sets it, so reading it at once can see it empty.
 */
const statusLine = (text: string) =>
  waitFor(() => expect(screen.getByRole("status").textContent).toBe(text));

const card = (title: string) => {
  const found = Array.from(
    document.querySelectorAll<HTMLElement>("[data-slot=kanban-card]"),
  ).find((node) => node.textContent?.includes(title));
  if (!found) throw new Error(`no card titled ${title}`);
  return found;
};
const titlesIn = (label: string) =>
  cardsIn(label).map(
    (c) => c.querySelector("[data-slot=ticket-title]")?.textContent,
  );
const dragTo = (title: string, label: string) => dragCardTo(card(title), label);
const status = () => screen.getByRole("status");

beforeEach(() => {
  channels.length = 0;
  useBoard.getState().reset();
  stubBoardDragDom(["draft", "ready", "building", "review", "done"]);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("the Board panel", () => {
  it("shows the five columns, each with the tickets of the board view, their number, title and priority label", async () => {
    await open();

    expect(
      screen.getAllByRole("group").map((g) => g.getAttribute("aria-label")),
    ).toEqual(["Draft", "Ready", "Building", "Review", "Done"]);
    expect(titlesIn("Draft")).toEqual(["Sketch the graph"]);
    expect(titlesIn("Ready")).toEqual(["Wire the poller"]);
    expect(titlesIn("Building")).toEqual(["Open the draft"]);
    expect(titlesIn("Review")).toEqual(["Read the board"]);
    expect(titlesIn("Done")).toEqual(["Lift the board"]);
    const ready = within(card("Wire the poller"));
    expect(ready.getByText("#12")).toBeTruthy();
    expect(ready.getByText("P1")).toBeTruthy();
    expect(within(card("Open the draft")).getByText("P0")).toBeTruthy();
    expect(within(card("Sketch the graph")).queryByText(/^P\d/)).toBeNull();
  });

  it("watches the board once, and the watch ends when the panel unmounts", async () => {
    const view = render(<Board onOpenWorker={() => {}} />);
    await waitFor(() => expect(channels).toHaveLength(1));

    view.unmount();
    await emitBoard();

    expect(boardWatch).toHaveBeenCalledTimes(1);
    expect(useBoard.getState().view).toBeNull();
  });

  it("only Ready tickets carry Start", async () => {
    await open();

    expect(screen.getAllByRole("button", { name: /^Start/ })).toHaveLength(1);
    expect(
      within(card("Wire the poller")).getByRole("button", {
        name: "Start #12",
      }),
    ).toBeTruthy();
  });

  it("Start calls workerStart with the ticket's id", async () => {
    await open();

    fireEvent.click(screen.getByRole("button", { name: "Start #12" }));

    await waitFor(() => expect(workerStart).toHaveBeenCalledWith("12"));
    expect(workerStart).toHaveBeenCalledTimes(1);
  });

  it("a successful Start calls onOpenWorker with the ticket's id and title", async () => {
    const { onOpenWorker } = await open();

    fireEvent.click(screen.getByRole("button", { name: "Start #12" }));

    await waitFor(() =>
      expect(onOpenWorker).toHaveBeenCalledWith({
        id: "12",
        title: "Wire the poller",
      }),
    );
    expect(onOpenWorker).toHaveBeenCalledTimes(1);
  });

  it("a Start that fails or waits for a slot opens no worker", async () => {
    workerStart.mockResolvedValueOnce(err("The ticket is closed."));
    workerStart.mockResolvedValueOnce(ok({ ticket: "12", waiting: true }));
    const { onOpenWorker } = await open();

    fireEvent.click(screen.getByRole("button", { name: "Start #12" }));
    await screen.findByText("The ticket is closed.");
    fireEvent.click(screen.getByRole("button", { name: "Start #12" }));
    await screen.findByText("#12 waits for a slot.");

    expect(onOpenWorker).not.toHaveBeenCalled();
  });

  it("a Start that fails shows its error line", async () => {
    workerStart.mockResolvedValueOnce(err("The ticket is closed."));
    await open();

    fireEvent.click(screen.getByRole("button", { name: "Start #12" }));

    await statusLine("The ticket is closed.");
  });

  it("a Start error stays in the line when a move that snaps back answers after it", async () => {
    let answer: (view: Ok<MoveView>) => void = () => {};
    boardMove.mockImplementationOnce(
      () => new Promise((resolve) => (answer = resolve)),
    );
    workerStart.mockResolvedValueOnce(err("The ticket is closed."));
    await open();

    dragTo("Sketch the graph", "Ready");
    await waitFor(() => expect(boardMove).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole("button", { name: "Start #12" }));
    await waitFor(() =>
      expect(status().textContent).toContain("The ticket is closed."),
    );
    await act(async () =>
      answer(ok({ kind: "snap_back", line: "Not that way." })),
    );

    expect(status().textContent).toContain("The ticket is closed.");
    expect(status().textContent).toContain("Not that way.");
  });

  it("a Start that waits for a slot says so", async () => {
    workerStart.mockResolvedValueOnce(ok({ ticket: "12", waiting: true }));
    await open();

    fireEvent.click(screen.getByRole("button", { name: "Start #12" }));

    await statusLine("#12 waits for a slot.");
  });

  it("Start does not open the ticket", async () => {
    await open();

    fireEvent.click(screen.getByRole("button", { name: "Start #12" }));
    await settle();

    expect(openLink).not.toHaveBeenCalled();
  });

  it("a drag from Ready to Building calls boardMove and shows the ticket in Building", async () => {
    let answer: (view: Ok<MoveView>) => void = () => {};
    boardMove.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          answer = resolve;
        }),
    );
    await open();

    await dragTo("Wire the poller", "Building");

    await waitFor(() =>
      expect(boardMove).toHaveBeenCalledWith("12", "building"),
    );
    await waitFor(() =>
      expect(titlesIn("Building")).toEqual([
        "Wire the poller",
        "Open the draft",
      ]),
    );
    expect(titlesIn("Ready")).toEqual([]);
    await act(async () => answer(ok({ kind: "started", waiting: false })));
    expect(titlesIn("Building")).toEqual(["Wire the poller", "Open the draft"]);
  });

  it("asks GitHub for the board again once a drag started a worker", async () => {
    boardMove.mockResolvedValueOnce(ok({ kind: "started", waiting: false }));
    await open();

    await dragTo("Wire the poller", "Building");

    await waitFor(() => expect(boardRefresh).toHaveBeenCalledTimes(1));
  });

  it("a snap back puts the ticket back and shows the answer's line", async () => {
    boardMove.mockResolvedValueOnce(
      ok({ kind: "snap_back", line: "No slot, and none will come." }),
    );
    await open();

    await dragTo("Wire the poller", "Building");

    await statusLine("No slot, and none will come.");
    await waitFor(() => expect(titlesIn("Ready")).toEqual(["Wire the poller"]));
    expect(titlesIn("Building")).toEqual(["Open the draft"]);
  });

  it("a drag that waits for a slot puts the ticket back and says it waits", async () => {
    boardMove.mockResolvedValueOnce(ok({ kind: "started", waiting: true }));
    await open();

    await dragTo("Wire the poller", "Building");

    await statusLine("#12 waits for a slot.");
    await waitFor(() => expect(titlesIn("Ready")).toEqual(["Wire the poller"]));
  });

  it("a drag that fails puts the ticket back and shows the error line", async () => {
    boardMove.mockResolvedValueOnce(err("Ticket 12 is not on the board."));
    await open();

    await dragTo("Wire the poller", "Building");

    await statusLine("Ticket 12 is not on the board.");
    await waitFor(() => expect(titlesIn("Ready")).toEqual(["Wire the poller"]));
  });

  it("a drag from Draft to Ready snaps back with the line the move table gives", async () => {
    const line =
      "Writing the ready label waits for Phase 3: mark the ticket ready on GitHub.";
    boardMove.mockResolvedValueOnce(ok({ kind: "snap_back", line }));
    await open();

    await dragTo("Sketch the graph", "Ready");

    expect(boardMove).toHaveBeenCalledWith("11", "ready");
    await statusLine(line);
    await waitFor(() =>
      expect(titlesIn("Draft")).toEqual(["Sketch the graph"]),
    );
    expect(titlesIn("Ready")).toEqual(["Wire the poller"]);
  });

  it("the Move to menu does the same by keyboard", async () => {
    boardMove.mockResolvedValueOnce(
      ok({
        kind: "snap_back",
        line: "A ticket's column follows its worker and its issue.",
      }),
    );
    await open();

    const trigger = within(card("Lift the board")).getByRole("button", {
      name: "Move Lift the board",
    });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: "Enter" });
    fireEvent.click(await screen.findByRole("menuitem", { name: "Draft" }));

    await waitFor(() => expect(boardMove).toHaveBeenCalledWith("15", "draft"));
    await statusLine("A ticket's column follows its worker and its issue.");
    await waitFor(() => expect(titlesIn("Done")).toEqual(["Lift the board"]));
  });

  it("the Move to menu moves a Ready ticket to Building like a drop", async () => {
    boardMove.mockResolvedValueOnce(ok({ kind: "started", waiting: false }));
    await open();

    const trigger = within(card("Wire the poller")).getByRole("button", {
      name: "Move Wire the poller",
    });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: "Enter" });
    fireEvent.click(await screen.findByRole("menuitem", { name: "Building" }));

    await waitFor(() =>
      expect(boardMove).toHaveBeenCalledWith("12", "building"),
    );
    await waitFor(() =>
      expect(titlesIn("Building")).toEqual([
        "Wire the poller",
        "Open the draft",
      ]),
    );
  });

  it("clicking a Building ticket calls onOpenWorker with its id and title", async () => {
    const { onOpenWorker } = await open();

    fireEvent.click(card("Open the draft"));

    expect(onOpenWorker).toHaveBeenCalledWith({
      id: "13",
      title: "Open the draft",
    });
    expect(openLink).not.toHaveBeenCalled();
  });

  it("clicking a Review ticket opens its pull request through openLink", async () => {
    const { onOpenWorker } = await open();

    fireEvent.click(card("Read the board"));

    await waitFor(() =>
      expect(openLink).toHaveBeenCalledWith("https://github.com/o/r/pull/40"),
    );
    expect(onOpenWorker).not.toHaveBeenCalled();
  });

  it("clicking any other ticket opens its issue", async () => {
    await open();

    fireEvent.click(card("Sketch the graph"));
    fireEvent.click(card("Lift the board"));

    await waitFor(() => expect(openLink).toHaveBeenCalledTimes(2));
    expect(openLink).toHaveBeenNthCalledWith(
      1,
      "https://github.com/o/r/issues/11",
    );
    expect(openLink).toHaveBeenNthCalledWith(
      2,
      "https://github.com/o/r/issues/15",
    );
  });

  it("a Review ticket with no pull request opens its issue", async () => {
    await open({
      held_until: null,
      needs_you: 0,
      tickets: [ticket("14", "Read the board", "review")],
    });

    fireEvent.click(card("Read the board"));

    await waitFor(() =>
      expect(openLink).toHaveBeenCalledWith("https://github.com/o/r/issues/14"),
    );
  });

  it("a link nothing opened is copied, and the line says why", async () => {
    const writeText = vi.fn(async () => {});
    vi.stubGlobal("navigator", { clipboard: { writeText } });
    openLink.mockResolvedValueOnce(
      ok({
        kind: "copy",
        url: "https://github.com/o/r/issues/11",
        message: "This system has no browser to open it: the link is copied.",
      }),
    );
    await open();

    fireEvent.click(card("Sketch the graph"));

    await statusLine(
      "This system has no browser to open it: the link is copied.",
    );
    expect(writeText).toHaveBeenCalledWith("https://github.com/o/r/issues/11");
  });

  it("a link that cannot be opened shows the error line", async () => {
    openLink.mockImplementationOnce(async () => {
      await new Promise((r) => setTimeout(r, 50));
      return err("No project is picked.");
    });
    await open();

    fireEvent.click(card("Sketch the graph"));

    await statusLine("No project is picked.");
  });

  it("a held limit shows when it lifts, with the board still there", async () => {
    await open({ ...VIEW, held_until: 1_794_000_000 });

    const limit = screen.getByText(/^GitHub is limiting kelpie until/);
    expect(limit.textContent).toMatch(/until \d{1,2}:\d{2}/);
    expect(limit.querySelector("time")?.dateTime).toBe(
      "2026-11-06T21:20:00.000Z",
    );
    expect(titlesIn("Ready")).toEqual(["Wire the poller"]);
  });

  it("shows no limit when none is held", async () => {
    await open();

    expect(screen.queryByText(/GitHub is limiting kelpie/)).toBeNull();
  });

  it("Unavailable shows its message in place of the board", async () => {
    render(<Board onOpenWorker={() => {}} />);
    await waitFor(() => expect(channels).toHaveLength(1));

    await emit({
      kind: "unavailable",
      message: "gh is not installed. Install it and run gh auth login.",
    });

    expect(status().textContent).toBe(
      "gh is not installed. Install it and run gh auth login.",
    );
    expect(screen.queryByRole("group")).toBeNull();
  });

  it("the board comes back when a board follows Unavailable", async () => {
    await open();
    await emit({ kind: "unavailable", message: "Offline." });
    expect(screen.queryByRole("group")).toBeNull();

    await emitBoard();

    expect(screen.getAllByRole("group")).toHaveLength(5);
    expect(screen.queryByText("Offline.")).toBeNull();
  });

  it("a watch that fails shows its error in place of the board", async () => {
    boardWatch.mockResolvedValueOnce(err("No project is picked."));

    render(<Board onOpenWorker={() => {}} />);

    await statusLine("No project is picked.");
    expect(screen.queryByRole("group")).toBeNull();
  });

  it("a new board view replaces the tickets", async () => {
    await open();

    await emitBoard({
      held_until: null,
      needs_you: 0,
      tickets: [ticket("12", "Wire the poller", "building")],
    });

    expect(titlesIn("Ready")).toEqual([]);
    expect(titlesIn("Building")).toEqual(["Wire the poller"]);
  });

  it("a watch the panel closed before it was sent is never sent", async () => {
    render(
      <StrictMode>
        <Board onOpenWorker={() => {}} />
      </StrictMode>,
    );

    await waitFor(() => expect(boardWatch).toHaveBeenCalledTimes(1));
    await settle();
    expect(boardWatch).toHaveBeenCalledTimes(1);
    await emitBoard();
    expect(titlesIn("Ready")).toEqual(["Wire the poller"]);
  });

  it("a watch waits for the one before to answer, so the later poller stays", async () => {
    const answers: (() => void)[] = [];
    boardWatch.mockImplementationOnce(
      (channel: Channel<BoardEvent>) =>
        new Promise((resolve) => {
          channels.push(channel);
          answers.push(() => resolve(ok(null)));
        }),
    );
    useBoard.getState().watch();
    await waitFor(() => expect(answers).toHaveLength(1));
    useBoard.getState().watch();
    await settle();

    expect(boardWatch).toHaveBeenCalledTimes(1);
    answers[0]?.();
    await waitFor(() => expect(boardWatch).toHaveBeenCalledTimes(2));
    expect(channels).toHaveLength(2);
  });
});
