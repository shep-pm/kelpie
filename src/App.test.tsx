import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/bindings", () => ({
  commands: { projectGet: async () => "/work/project" },
}));

import { App } from "./App";

afterEach(cleanup);

describe("App", () => {
  it("shows the PM's chat box", async () => {
    render(<App />);
    expect(await screen.findByPlaceholderText("Message the PM")).toBeTruthy();
  });
});
