import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SandboxRequest } from "./SandboxRequest";
import type { SandboxRequest as Asked } from "./store";

/** A request waiting for the maintainer, as the store keeps it. */
const waiting: Asked = {
  id: "net-1",
  host: "docs.rs",
  why: "To read the tokio::process docs for the reaper ticket.",
  answer: null,
};

describe("the sandbox request", () => {
  afterEach(cleanup);

  it("says the host the PM wants and why", () => {
    render(<SandboxRequest request={waiting} onAnswer={() => {}} />);

    expect(screen.getByText("PM wants to reach docs.rs")).toBeTruthy();
    expect(
      screen.getByText(
        "To read the tokio::process docs for the reaper ticket.",
      ),
    ).toBeTruthy();
  });

  it("each action answers with its own choice", () => {
    const onAnswer = vi.fn();
    render(<SandboxRequest request={waiting} onAnswer={onAnswer} />);

    fireEvent.click(screen.getByRole("button", { name: "Allow once" }));
    fireEvent.click(
      screen.getByRole("button", { name: "Allow for this project" }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Deny" }));

    expect(onAnswer.mock.calls).toEqual([["once"], ["project"], ["deny"]]);
  });

  it("an answered request collapses to one line with no buttons", () => {
    render(
      <SandboxRequest
        request={{ ...waiting, answer: "project" }}
        onAnswer={() => {}}
      />,
    );

    expect(screen.queryByRole("button")).toBeNull();
    expect(screen.getByText(/docs\.rs allowed for this project/)).toBeTruthy();
    expect(screen.getByText("Undo in settings")).toBeTruthy();
  });

  it("an answer left waiting when the turn ended reads as denied", () => {
    render(
      <SandboxRequest
        request={{ ...waiting, answer: "deny" }}
        onAnswer={() => {}}
      />,
    );

    expect(screen.getByText("docs.rs denied")).toBeTruthy();
  });

  it("an answer that could not be saved says what went wrong", () => {
    const problem = "docs.rs is allowed for this session, but not saved";
    render(
      <SandboxRequest
        request={{ ...waiting, answer: "project", error: problem }}
        onAnswer={() => {}}
      />,
    );

    expect(screen.getByText(problem)).toBeTruthy();
    expect(screen.queryByText("Undo in settings")).toBeNull();
  });

  it("requests waiting behind the open one are counted, not stacked", () => {
    render(<SandboxRequest request={waiting} queued={2} onAnswer={() => {}} />);

    expect(screen.getByText("+2 more")).toBeTruthy();
    expect(screen.getAllByRole("button")).toHaveLength(3);
  });
});
