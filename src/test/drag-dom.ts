/**
 * What dnd-kit needs of a browser and jsdom lacks. A drag test stubs
 * `PointerEvent`, `IntersectionObserver` and `ResizeObserver` with `vi.stubGlobal`, calls
 * `stubAnimations()` and `stubPointerCapture()`, and stubs `getBoundingClientRect` for its own layout.
 */

import { vi } from "vitest";

/** A `PointerEvent` that jsdom can construct: a mouse event with a pointer id. */
export class StubPointerEvent extends MouseEvent {
  readonly pointerId: number;
  readonly pointerType: string;
  readonly isPrimary: boolean;
  constructor(type: string, init: PointerEventInit = {}) {
    super(type, init);
    this.pointerId = init.pointerId ?? 1;
    this.pointerType = init.pointerType ?? "mouse";
    this.isPrimary = init.isPrimary ?? true;
  }
}

/** What dnd-kit's drop animation needs of an Animation: it finishes at once. */
export class StubAnimation extends EventTarget implements Animation {
  currentTime: CSSNumberish | null = null;
  effect: AnimationEffect | null = null;
  finished: Promise<Animation> = Promise.resolve(this);
  id = "";
  oncancel = null;
  onfinish = null;
  onremove = null;
  overallProgress: number | null = null;
  pending = false;
  playState: AnimationPlayState = "finished";
  playbackRate = 1;
  ready: Promise<Animation> = Promise.resolve(this);
  replaceState: AnimationReplaceState = "active";
  startTime: CSSNumberish | null = null;
  timeline: AnimationTimeline | null = null;
  cancel() {}
  commitStyles() {}
  finish() {}
  pause() {}
  persist() {}
  play() {}
  reverse() {}
  updatePlaybackRate() {}
}

/** An observer that never reports. */
export class StubIntersectionObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
  takeRecords() {
    return [];
  }
}

/** An observer that never reports. dnd-kit measures a held card with one. */
export class StubResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}

/** Gives jsdom the animation calls dnd-kit makes: none are running, `animate` finishes at once, and nothing asks for reduced motion. */
export function stubAnimations(): void {
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener() {},
    removeEventListener() {},
    addListener() {},
    removeListener() {},
    dispatchEvent: () => false,
  }));
  document.getAnimations = () => [];
  Element.prototype.getAnimations = () => [];
  Element.prototype.animate = () => new StubAnimation();
}

/** Gives jsdom the pointer capture calls a drag makes, which it has not got. */
export function stubPointerCapture(): void {
  Element.prototype.setPointerCapture = () => {};
  Element.prototype.releasePointerCapture = () => {};
  Element.prototype.hasPointerCapture = () => false;
}

/** How a test lays out a list for dnd-kit: rows stack `rowHeight` apart in document order. */
export interface ListLayout {
  rowSelector: string;
  rowHeight: number;
}

const ROW_WIDTH = 300;

/**
 * Where jsdom, which lays nothing out, puts `el`: a row, or anything inside
 * one, takes its row's slot, and the rest fills a 1024 x 768 window. A held row
 * moves with a CSS translate, which is added here.
 */
export function listRowRect(el: Element, layout: ListLayout): DOMRect {
  const row = el.closest(layout.rowSelector);
  const rows = Array.from(document.querySelectorAll(layout.rowSelector));
  const [x, y, width, height] = row
    ? [0, rows.indexOf(row) * layout.rowHeight, ROW_WIDTH, layout.rowHeight]
    : [0, 0, 1024, 768];
  const [dx = 0, dy = 0] = (
    (el instanceof HTMLElement &&
      el.style.getPropertyValue("--dnd-translate")) ||
    ""
  )
    .split(" ")
    .map((n) => Number.parseFloat(n) || 0);
  return new DOMRect(x + dx, y + dy, width, height);
}

/** The centre of `el` under `layout`. */
export function listCentre(
  el: Element,
  layout: ListLayout,
): { x: number; y: number } {
  const { x, y, width, height } = listRowRect(el, layout);
  return { x: x + width / 2, y: y + height / 2 };
}

/**
 * Stubs what dnd-kit needs for a list laid out by `layout`: `PointerEvent`,
 * `IntersectionObserver`, animations, `getBoundingClientRect` and
 * `elementFromPoint`. Call `vi.unstubAllGlobals()` and `vi.restoreAllMocks()`
 * after the test.
 */
export function stubListDragDom(layout: ListLayout): void {
  vi.stubGlobal("PointerEvent", StubPointerEvent);
  vi.stubGlobal("IntersectionObserver", StubIntersectionObserver);
  vi.stubGlobal("ResizeObserver", StubResizeObserver);
  stubAnimations();
  stubPointerCapture();
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(
    function (this: Element) {
      return listRowRect(this, layout);
    },
  );
  document.elementFromPoint = (_x: number, y: number) =>
    document.querySelectorAll<HTMLElement>(layout.rowSelector)[
      Math.floor(y / layout.rowHeight)
    ] ?? document.body;
}

/** A primary mouse `PointerEvent` at `at`, for `fireEvent`. Needs `stubListDragDom`. */
export function mousePointerEvent(
  type: "pointerdown" | "pointermove" | "pointerup",
  at: { x: number; y: number },
): PointerEvent {
  return new PointerEvent(type, {
    bubbles: true,
    cancelable: true,
    clientX: at.x,
    clientY: at.y,
    pointerType: "mouse",
    pointerId: 1,
    isPrimary: true,
    button: 0,
  });
}
