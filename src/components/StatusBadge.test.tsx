import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { STATUS_WORDS, type Status, StatusBadge } from "./StatusBadge";

const STATUSES: Status[] = [
  "idle",
  "running",
  "needs",
  "blocked",
  "done",
  "failed",
];

afterEach(cleanup);

describe("the status badge", () => {
  it.each(STATUSES)("shows %s with its word and its glyph", (status) => {
    const { container } = render(<StatusBadge status={status} />);
    const glyph = container.querySelector("svg");

    expect(container.textContent?.trim()).toBe(STATUS_WORDS[status]);
    expect(glyph?.getAttribute("data-status")).toBe(status);
  });

  it("gives every status a glyph of its own, so the state reads without colour", () => {
    const drawn = new Set<string>();
    for (const status of STATUSES) {
      const { container } = render(<StatusBadge status={status} />);
      const glyph = container.querySelector("svg");
      if (!glyph) throw new Error(`${status} has no glyph`);
      drawn.add(glyph.innerHTML);
      cleanup();
    }
    expect(drawn.size).toBe(STATUSES.length);
  });

  it("takes a shorter word in place of the status's own", () => {
    const { container } = render(
      <StatusBadge status="needs">Paused</StatusBadge>,
    );
    const glyph = container.querySelector("svg");

    expect(container.textContent?.trim()).toBe("Paused");
    expect(glyph?.getAttribute("data-status")).toBe("needs");
  });

  it("takes the colour from the status, never from the caller", () => {
    const { container } = render(<StatusBadge status="blocked" />);
    const badge = container.querySelector("[data-slot=status-badge]");

    expect(badge?.className).toContain("bg-status-blocked-soft");
    expect(badge?.className).toContain("text-status-blocked-fg");
    expect(container.querySelector("svg")?.getAttribute("class")).toContain(
      "text-status-blocked",
    );
  });
});
