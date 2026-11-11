/**
 * `<KanbanBoard>`: tickets as columns, with a drag and a Move to menu that ask
 * `onMove` where a ticket goes. Lifted from chelone's `kanban-board.test.tsx`
 * (`eb149dee4`). A drag is driven with pointer events; jsdom has no
 * `PointerEvent`, element geometry or `elementFromPoint`, so the test stubs
 * those and dnd-kit runs unmocked.
 */
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  cardArea,
  cardsIn,
  centre,
  column,
  dragTo as dragCardTo,
  insideArea,
  moveOver,
  pickUp as pickUpCard,
  pointer,
  settle,
  stubBoardDragDom,
  wait,
} from "@/test/board-drag";
import {
  KanbanBoard,
  type KanbanColumn,
  type MoveOutcome,
} from "./KanbanBoard";

afterEach(cleanup);

type Id = "todo" | "doing" | "done";
type Task = { id: string; title: string; column: Id };

const TASKS: Task[] = [
  { id: "1", title: "Write it", column: "todo" },
  { id: "2", title: "Ship it", column: "doing" },
  { id: "3", title: "Test it", column: "todo" },
];

const COLUMNS: KanbanColumn<Id>[] = [
  { id: "todo", label: "To do" },
  { id: "doing", label: "In progress" },
  { id: "done", label: "Done" },
];

const moved: MoveOutcome = { kind: "moved" };

type Props = {
  tickets?: Task[];
  onMove?: (ticket: Task, to: Id) => Promise<MoveOutcome>;
  onSelect?: (ticket: Task) => void;
  renderCard?: (ticket: Task) => ReactNode;
  notice?: string | null;
};

function board({ tickets = TASKS, onMove, ...rest }: Props = {}) {
  return (
    <KanbanBoard
      columns={COLUMNS}
      tickets={tickets}
      onMove={onMove ?? (async () => moved)}
      {...rest}
    />
  );
}

function mount(props: Props = {}) {
  return render(board(props));
}

const titleOf = (node: Element) =>
  node.querySelector("[data-slot=kanban-card-title]")?.textContent;
const cardTitles = (label: string) => cardsIn(label).map(titleOf);
const card = (title: string) => {
  const found = Array.from(
    document.querySelectorAll<HTMLElement>("[data-slot=kanban-card]"),
  ).find((node) => titleOf(node) === title);
  if (!found) throw new Error(`no card titled ${title}`);
  return found;
};
const cardWithId = (id: string) => {
  const found = document.querySelector<HTMLElement>(`[data-ticket-id="${id}"]`);
  if (!found) throw new Error(`no card with id ${id}`);
  return found;
};

beforeEach(() => stubBoardDragDom(COLUMNS.map((c) => c.id)));

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

/**
 * Wait for the board's status line to say `text`. The line is always on the
 * page, empty until a move or a notice sets it, so reading it at once can see
 * it empty, and how long `onMove` takes to answer depends on the machine.
 */
const statusLine = (text: string) =>
  waitFor(() => expect(screen.getByRole("status").textContent).toBe(text));

const pickUp = (title: string, pointerType: "mouse" | "touch" = "mouse") =>
  pickUpCard(card(title), pointerType);
const dragTo = (
  title: string,
  label: string,
  pointerType: "mouse" | "touch" = "mouse",
) => dragCardTo(card(title), label, pointerType);

describe("<KanbanBoard>", () => {
  it("renders a column per definition, in order, with its label and ticket count", () => {
    mount();

    const groups = screen.getAllByRole("group");
    expect(groups.map((g) => g.getAttribute("data-column-id"))).toEqual([
      "todo",
      "doing",
      "done",
    ]);
    expect(groups.every((g) => g.dataset.slot === "kanban-column")).toBe(true);
    const counts = groups.map(
      (g) => g.querySelector("[data-slot=kanban-column-count]")?.textContent,
    );
    expect(counts).toEqual(["2", "1", "0"]);
  });

  it("puts each ticket in its column, in the order it is given", () => {
    mount();

    expect(cardTitles("To do")).toEqual(["Write it", "Test it"]);
    expect(cardTitles("In progress")).toEqual(["Ship it"]);
    expect(card("Write it").getAttribute("data-ticket-id")).toBe("1");
  });

  it("shows no card for a ticket whose column is not listed", () => {
    mount({
      tickets: [...TASKS, { id: "4", title: "Lost it", column: "gone" as Id }],
    });

    expect(screen.queryByText("Lost it")).toBeNull();
  });

  it("an empty column says it has no tickets", () => {
    mount();

    expect(within(column("Done")).getByText("No tickets").dataset.slot).toBe(
      "kanban-empty",
    );
    expect(within(column("To do")).queryByText("No tickets")).toBeNull();
  });

  it("renderCard replaces the card content", () => {
    mount({
      renderCard: (ticket) => <em data-testid="custom">{`#${ticket.id}`}</em>,
    });

    expect(
      screen.getAllByTestId("custom").map((node) => node.textContent),
    ).toEqual(["#1", "#3", "#2"]);
    expect(screen.queryByText("Write it")).toBeNull();
  });

  it("dropping a card on another column calls onMove with the ticket and the column", async () => {
    const onMove = vi.fn(async () => moved);
    mount({ onMove });

    await dragTo("Write it", "In progress");

    await waitFor(() => expect(onMove).toHaveBeenCalledTimes(1));
    expect(onMove).toHaveBeenCalledWith(TASKS[0], "doing");
  });

  it("the card changes column before onMove answers", async () => {
    let answer: (outcome: MoveOutcome) => void = () => {};
    const onMove = vi.fn(
      () =>
        new Promise<MoveOutcome>((resolve) => {
          answer = resolve;
        }),
    );
    mount({ onMove });

    await dragTo("Write it", "In progress");

    await waitFor(() =>
      expect(cardTitles("In progress")).toEqual(["Write it", "Ship it"]),
    );
    expect(cardTitles("To do")).toEqual(["Test it"]);
    answer(moved);
  });

  it("a card that moved stays in its new column until the board's tickets change", async () => {
    const view = mount();

    await dragTo("Write it", "In progress");
    await settle();
    expect(cardTitles("In progress")).toEqual(["Write it", "Ship it"]);

    const next: Task[] = [
      { id: "1", title: "Write it", column: "doing" },
      { id: "2", title: "Ship it", column: "done" },
      { id: "3", title: "Test it", column: "todo" },
    ];
    view.rerender(board({ tickets: next }));
    expect(cardTitles("In progress")).toEqual(["Write it"]);
    expect(cardTitles("Done")).toEqual(["Ship it"]);
  });

  it("a snap back puts the card back and shows the line the answer carries", async () => {
    const onMove = vi.fn(
      async (): Promise<MoveOutcome> => ({
        kind: "snap_back",
        line: "Ready first: nothing starts from Draft.",
      }),
    );
    mount({ onMove });

    await dragTo("Write it", "In progress");

    await statusLine("Ready first: nothing starts from Draft.");
    await waitFor(() =>
      expect(cardTitles("To do")).toEqual(["Write it", "Test it"]),
    );
    expect(cardTitles("In progress")).toEqual(["Ship it"]);
  });

  it("a rejected move puts the card back and shows the error's message", async () => {
    const onMove = vi.fn(async (): Promise<MoveOutcome> => {
      throw new Error("the window lost the engine");
    });
    mount({ onMove });

    await dragTo("Write it", "In progress");

    await statusLine("the window lost the engine");
    await waitFor(() =>
      expect(cardTitles("To do")).toEqual(["Write it", "Test it"]),
    );
  });

  it("the status line clears when the next move starts", async () => {
    const outcomes: MoveOutcome[] = [{ kind: "snap_back", line: "No." }, moved];
    const onMove = vi.fn(async () => outcomes.shift() ?? moved);
    mount({ onMove });

    await dragTo("Write it", "In progress");
    await screen.findByText("No.");
    await waitFor(() =>
      expect(cardTitles("To do")).toEqual(["Write it", "Test it"]),
    );
    await dragTo("Write it", "Done");

    await waitFor(() => expect(screen.queryByText("No.")).toBeNull());
  });

  it("shows the notice a parent gives in the status line", () => {
    mount({ notice: "#5 waits for a slot." });

    expect(screen.getByRole("status").textContent).toBe("#5 waits for a slot.");
  });

  it("a notice that arrives replaces the line of an earlier move", async () => {
    const onMove = vi.fn(
      async (): Promise<MoveOutcome> => ({ kind: "snap_back", line: "No." }),
    );
    const view = mount({ onMove });
    await dragTo("Write it", "In progress");
    await screen.findByText("No.");

    view.rerender(board({ onMove, notice: "Opened." }));

    expect(screen.getByRole("status").textContent).toBe("Opened.");
  });

  it("a move's line that comes after a notice shows beside it", async () => {
    let answer: (outcome: MoveOutcome) => void = () => {};
    const onMove = vi.fn(
      () => new Promise<MoveOutcome>((resolve) => (answer = resolve)),
    );
    const view = mount({ onMove });
    await dragTo("Write it", "In progress");
    await waitFor(() => expect(onMove).toHaveBeenCalledTimes(1));

    view.rerender(board({ onMove, notice: "Start failed." }));
    await act(async () => answer({ kind: "snap_back", line: "No." }));

    expect(screen.getByRole("status").textContent).toBe("No. Start failed.");
  });

  it("dropping a card on its own column asks nothing", async () => {
    const onMove = vi.fn(async () => moved);
    mount({ onMove });

    await dragTo("Write it", "To do");

    expect(onMove).not.toHaveBeenCalled();
    expect(cardTitles("To do")).toEqual(["Write it", "Test it"]);
  });

  it("a drag that ends outside a column asks nothing and clears the highlight", async () => {
    const onMove = vi.fn(async () => moved);
    mount({ onMove });

    await pickUp("Write it");
    await moveOver("Done");
    expect(cardArea("Done").hasAttribute("data-over")).toBe(true);
    // Past the last column nothing is under the pointer.
    pointer(document, "pointermove", { x: 2000, y: 2000 });
    await waitFor(() =>
      expect(cardArea("Done").hasAttribute("data-over")).toBe(false),
    );
    pointer(document, "pointerup", { x: 2000, y: 2000 });
    await settle();

    expect(card("Write it").hasAttribute("data-dragging")).toBe(false);
    expect(onMove).not.toHaveBeenCalled();
    expect(cardTitles("To do")).toEqual(["Write it", "Test it"]);
  });

  it("marks the held card and the column under it while dragging", async () => {
    mount();

    await pickUp("Write it");
    expect(card("Write it").hasAttribute("data-dragging")).toBe(true);

    await moveOver("Done");
    expect(cardArea("Done").hasAttribute("data-over")).toBe(true);
    expect(cardArea("In progress").hasAttribute("data-over")).toBe(false);

    await moveOver("In progress");
    expect(cardArea("Done").hasAttribute("data-over")).toBe(false);
    expect(cardArea("In progress").hasAttribute("data-over")).toBe(true);
  });

  it("a touch drag moves a card to another column", async () => {
    const onMove = vi.fn(async () => moved);
    mount({ onMove });

    await dragTo("Write it", "Done", "touch");

    await waitFor(() => expect(onMove).toHaveBeenCalledTimes(1));
    expect(onMove).toHaveBeenCalledWith(TASKS[0], "done");
  });

  it("a touch that moves before the hold ends scrolls and starts no drag", async () => {
    const onMove = vi.fn(async () => moved);
    mount({ onMove });

    const from = centre(card("Write it"));
    pointer(card("Write it"), "pointerdown", from, "touch");
    pointer(document, "pointermove", { x: from.x + 40, y: from.y }, "touch");
    await wait(300);
    expect(card("Write it").hasAttribute("data-dragging")).toBe(false);
    pointer(document, "pointerup", insideArea("Done"), "touch");
    await settle();

    expect(onMove).not.toHaveBeenCalled();
  });

  it("a click on a card selects it and starts no drag", async () => {
    const onMove = vi.fn(async () => moved);
    const onSelect = vi.fn();
    mount({ onMove, onSelect });

    const at = centre(card("Write it"));
    pointer(card("Write it"), "pointerdown", at);
    expect(card("Write it").hasAttribute("data-dragging")).toBe(false);
    pointer(document, "pointerup", at);
    fireEvent.click(card("Write it"));
    await settle();

    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onSelect).toHaveBeenCalledWith(TASKS[0]);
    expect(card("Write it").hasAttribute("data-dragging")).toBe(false);
    expect(onMove).not.toHaveBeenCalled();
  });

  it("a press on the Move to button starts no drag", async () => {
    mount();

    const trigger = within(card("Write it")).getByRole("button", {
      name: "Move Write it",
    });
    const at = centre(card("Write it"));
    pointer(trigger, "pointerdown", at);
    pointer(document, "pointermove", { x: at.x + 40, y: at.y });
    await settle();

    expect(card("Write it").hasAttribute("data-dragging")).toBe(false);
  });

  it("a press on a link in renderCard starts no drag", async () => {
    mount({
      renderCard: (ticket) => (
        <a href="#x" data-testid={`link-${ticket.id}`}>
          {ticket.title}
        </a>
      ),
    });

    const at = centre(cardWithId("1"));
    pointer(screen.getByTestId("link-1"), "pointerdown", at);
    pointer(document, "pointermove", { x: at.x + 40, y: at.y });
    await settle();

    expect(document.querySelector("[data-dragging]")).toBeNull();
  });

  it("no card carries the native draggable attribute", () => {
    mount();

    for (const node of document.querySelectorAll("[data-slot=kanban-card]")) {
      expect(node.hasAttribute("draggable")).toBe(false);
    }
  });

  it("a card announces as a button, with no drag instructions", async () => {
    mount();
    await settle();

    const node = card("Write it");
    expect(node.getAttribute("role")).toBe("button");
    for (const name of [
      "aria-roledescription",
      "aria-describedby",
      "aria-pressed",
      "aria-grabbed",
    ]) {
      expect(node.hasAttribute(name), name).toBe(false);
    }
    expect(document.body.textContent).not.toContain("space bar");
  });

  it("the Move to menu lists the other columns and moving with the keyboard calls onMove like a drop", async () => {
    const onMove = vi.fn(async () => moved);
    mount({ onMove });

    const trigger = within(card("Write it")).getByRole("button", {
      name: "Move Write it",
    });
    expect(trigger.getAttribute("data-slot")).toBe("kanban-card-move");
    trigger.focus();
    fireEvent.keyDown(trigger, { key: "Enter" });
    const items = await screen.findAllByRole("menuitem");
    expect(items.map((item) => item.textContent)).toEqual([
      "In progress",
      "Done",
    ]);
    fireEvent.click(screen.getByRole("menuitem", { name: "Done" }));

    await waitFor(() => expect(onMove).toHaveBeenCalledTimes(1));
    expect(onMove).toHaveBeenCalledWith(TASKS[0], "done");
    await waitFor(() => expect(cardTitles("Done")).toEqual(["Write it"]));
  });

  it("the Move to menu puts a card back on a snap back, like a drop", async () => {
    const onMove = vi.fn(
      async (): Promise<MoveOutcome> => ({ kind: "snap_back", line: "No." }),
    );
    mount({ onMove });

    const trigger = within(card("Write it")).getByRole("button", {
      name: "Move Write it",
    });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: "Enter" });
    fireEvent.click(await screen.findByRole("menuitem", { name: "Done" }));

    await statusLine("No.");
    await waitFor(() =>
      expect(cardTitles("To do")).toEqual(["Write it", "Test it"]),
    );
  });

  it("an answer to an earlier move of a card does not undo a later move of it", async () => {
    const answers: ((outcome: MoveOutcome) => void)[] = [];
    const onMove = vi.fn(
      () => new Promise<MoveOutcome>((resolve) => answers.push(resolve)),
    );
    mount({ onMove });
    const moveWith = async (name: string, to: string) => {
      const trigger = within(card(name)).getByRole("button", {
        name: `Move ${name}`,
      });
      trigger.focus();
      fireEvent.keyDown(trigger, { key: "Enter" });
      fireEvent.click(await screen.findByRole("menuitem", { name: to }));
    };

    await moveWith("Write it", "In progress");
    await waitFor(() => expect(onMove).toHaveBeenCalledTimes(1));
    await moveWith("Write it", "Done");
    await waitFor(() => expect(onMove).toHaveBeenCalledTimes(2));
    await act(async () =>
      answers[0]?.({ kind: "snap_back", line: "Not that way." }),
    );

    expect(cardTitles("Done")).toEqual(["Write it"]);
    expect(screen.getByRole("status").textContent).toBe("");
    await act(async () => answers[1]?.(moved));
    expect(cardTitles("Done")).toEqual(["Write it"]);
  });

  it("the Move to button does not select the card", async () => {
    const onSelect = vi.fn();
    mount({ onSelect });

    const trigger = within(card("Write it")).getByRole("button", {
      name: "Move Write it",
    });
    fireEvent.click(trigger);
    fireEvent.keyDown(trigger, { key: "Enter" });
    fireEvent.click(
      await screen.findByRole("menuitem", { name: "In progress" }),
    );

    expect(onSelect).not.toHaveBeenCalled();
  });

  it("onSelect receives the ticket when a card is clicked or Enter or Space is pressed", () => {
    const onSelect = vi.fn();
    mount({ onSelect });

    fireEvent.click(card("Write it"));
    expect(onSelect).toHaveBeenLastCalledWith(TASKS[0]);
    fireEvent.keyDown(card("Ship it"), { key: "Enter" });
    expect(onSelect).toHaveBeenLastCalledWith(TASKS[1]);
    fireEvent.keyDown(card("Test it"), { key: " " });
    expect(onSelect).toHaveBeenLastCalledWith(TASKS[2]);
    expect(onSelect).toHaveBeenCalledTimes(3);
  });

  it("renders a title that holds markup as text", () => {
    const markup = "<img src=x onerror=alert(1)>";
    mount({ tickets: [{ id: "1", title: markup, column: "todo" }] });

    expect(screen.getByText(markup)).toBeTruthy();
    expect(document.querySelector("img")).toBeNull();
  });
});
