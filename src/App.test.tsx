import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { App } from "./App";

afterEach(cleanup);

describe("App", () => {
  it("renders the Kelpie heading", () => {
    render(<App />);
    const heading = screen.getByRole("heading", { level: 1, name: "Kelpie" });
    expect(heading.tagName).toBe("H1");
    expect(heading.textContent).toBe("Kelpie");
  });
});
