/**
 * Driving a drag on the board in jsdom, which lays nothing out. Columns sit
 * side by side and cards stack in each, so dnd-kit finds the card area under a
 * pointer. Kelpie's own, on top of `drag-dom.ts`: it knows the board's
 * `data-slot` names. Call `stubBoardDragDom` in `beforeEach`, and
 * `vi.unstubAllGlobals()` with `vi.restoreAllMocks()` in `afterEach`.
 */
import { fireEvent, getConfig, screen, waitFor } from "@testing-library/react";
import { expect, vi } from "vitest";
import {
  StubIntersectionObserver,
  StubPointerEvent,
  StubResizeObserver,
  stubAnimations,
  stubPointerCapture,
} from "./drag-dom";

const COLUMN_WIDTH = 280;
const COLUMN_STRIDE = 300;
const CARD_HEIGHT = 50;

type Point = { x: number; y: number };
type PointerKind = "mouse" | "touch";

function rect(x: number, y: number, width: number, height: number): DOMRect {
  return {
    x,
    y,
    width,
    height,
    left: x,
    top: y,
    right: x + width,
    bottom: y + height,
    toJSON: () => ({}),
  };
}

/** Where an element sits on the stubbed canvas: columns side by side, cards stacked. */
function rectOf(el: Element, columnIds: readonly string[]): DOMRect {
  const area = el.closest("[data-slot=kanban-column-cards]");
  const col = el.closest<HTMLElement>("[data-slot=kanban-column]");
  // dnd-kit clips a rect to its ancestors, so they cover the whole window.
  if (!area || !col) return rect(0, 0, 1024, 768);
  const x = columnIds.indexOf(col.dataset.columnId ?? "") * COLUMN_STRIDE;
  if (el === area) return rect(x, 0, COLUMN_WIDTH, 600);
  const cards = Array.from(area.querySelectorAll("[data-slot=kanban-card]"));
  const at = cards.findIndex((c) => c === el || c.contains(el));
  // dnd-kit moves the held card with a CSS translate, which jsdom does not lay out; it shows as a custom property.
  const translate =
    el instanceof HTMLElement
      ? el.style.getPropertyValue("--dnd-translate")
      : "";
  const [dx, dy] = (translate || "0px 0px")
    .split(" ")
    .map((n) => Number.parseFloat(n) || 0);
  return rect(
    x + 10 + (dx ?? 0),
    10 + at * (CARD_HEIGHT + 10) + (dy ?? 0),
    260,
    CARD_HEIGHT,
  );
}

/** Stub what dnd-kit needs for a board whose columns, left to right, are `columnIds`. */
export function stubBoardDragDom(columnIds: readonly string[]): void {
  vi.stubGlobal("PointerEvent", StubPointerEvent);
  vi.stubGlobal("IntersectionObserver", StubIntersectionObserver);
  vi.stubGlobal("ResizeObserver", StubResizeObserver);
  stubAnimations();
  stubPointerCapture();
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(
    function (this: Element) {
      return rectOf(this, columnIds);
    },
  );
  // Every pointer is over the card area of the column whose x range holds it.
  document.elementFromPoint = (x: number) =>
    document.querySelectorAll<HTMLElement>("[data-slot=kanban-column-cards]")[
      Math.floor(x / COLUMN_STRIDE)
    ] ?? document.body;
  columns = columnIds;
}

let columns: readonly string[] = [];

/** The column named `label`. */
export const column = (label: string) =>
  screen.getByRole("group", { name: label });

/** The card area of the column named `label`. */
export const cardArea = (label: string) => {
  const found = column(label).querySelector<HTMLElement>(
    "[data-slot=kanban-column-cards]",
  );
  if (!found) throw new Error(`no card area in ${label}`);
  return found;
};

/** The cards in the column named `label`. */
export const cardsIn = (label: string) =>
  Array.from(
    column(label).querySelectorAll<HTMLElement>("[data-slot=kanban-card]"),
  );

/**
 * Real time outside `act`, for asserting that nothing happens: React holds back
 * renders queued inside an `act` scope, and dnd-kit waits for a render.
 */
export const wait = (ms: number) =>
  getConfig().asyncWrapper(
    () => new Promise<void>((resolve) => setTimeout(resolve, ms)),
  );
export const settle = () => wait(100);

/** The centre of `el` on the stubbed canvas. */
export const centre = (el: Element): Point => {
  const { x, y, width, height } = rectOf(el, columns);
  return { x: x + width / 2, y: y + height / 2 };
};

/** A point inside a column's card area, off its exact centre (a zero distance has no ranking). */
export const insideArea = (label: string): Point => {
  const { x, y } = centre(cardArea(label));
  return { x: x + 20, y: y + 20 };
};

/** Fire a primary pointer event at `at`. */
export const pointer = (
  target: Element | Document,
  type: "pointerdown" | "pointermove" | "pointerup",
  at: Point,
  pointerType: PointerKind = "mouse",
) => {
  fireEvent(
    target,
    new PointerEvent(type, {
      bubbles: true,
      cancelable: true,
      clientX: at.x,
      clientY: at.y,
      pointerType,
      pointerId: 1,
      isPrimary: true,
      button: 0,
    }),
  );
};

/** Press a card and move past the activation distance; the card is then held. */
export async function pickUp(
  held: Element,
  pointerType: PointerKind = "mouse",
) {
  const from = centre(held);
  pointer(held, "pointerdown", from, pointerType);
  // A touch drag starts after its hold, a mouse drag after a few pixels.
  if (pointerType === "mouse") {
    pointer(
      document,
      "pointermove",
      { x: from.x + 10, y: from.y },
      pointerType,
    );
  }
  await waitFor(() =>
    expect(document.querySelector("[data-dragging]")).not.toBeNull(),
  );
}

/** Move the held card over a column's card area and wait for the highlight. */
export async function moveOver(
  label: string,
  pointerType: PointerKind = "mouse",
) {
  pointer(document, "pointermove", insideArea(label), pointerType);
  await waitFor(() =>
    expect(cardArea(label).hasAttribute("data-over")).toBe(true),
  );
}

/** Pick a card up and drop it on a column's card area. */
export async function dragTo(
  held: Element,
  label: string,
  pointerType: PointerKind = "mouse",
) {
  await pickUp(held, pointerType);
  await moveOver(label, pointerType);
  pointer(document, "pointerup", insideArea(label), pointerType);
  await waitFor(() =>
    expect(document.querySelector("[data-dragging]")).toBeNull(),
  );
}
