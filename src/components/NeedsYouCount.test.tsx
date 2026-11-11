import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { NeedsYouCount } from "./NeedsYouCount";

afterEach(cleanup);

describe("the needs-you count", () => {
  it("renders nothing at 0, since an empty badge is noise", () => {
    const { container } = render(<NeedsYouCount count={0} />);
    expect(container.textContent).toBe("");
  });

  it("shows the count", () => {
    const { container } = render(<NeedsYouCount count={5} />);
    expect(container.textContent).toBe("5");
  });

  it("shows 99+ above 99", () => {
    const { container } = render(<NeedsYouCount count={100} />);
    expect(container.textContent).toBe("99+");
  });

  it("is no target of its own: the tile, tab or item it sits on is clicked", () => {
    const { container } = render(<NeedsYouCount count={3} />);
    expect(container.querySelector("button,a,[role=button]")).toBeNull();
    expect(
      container.querySelector("span")?.getAttribute("tabindex"),
    ).toBeNull();
  });
});
