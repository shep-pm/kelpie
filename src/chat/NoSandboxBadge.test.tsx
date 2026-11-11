import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { NoSandboxBadge } from "./NoSandboxBadge";
import { useChat } from "./store";

describe("the No sandbox badge", () => {
  beforeEach(() => useChat.getState().reset());
  afterEach(cleanup);

  it("reads No sandbox while the session runs unsandboxed", () => {
    useChat.setState({
      sandbox: { refusal: "no_capabilities", unsandboxed: true },
    });
    render(<NoSandboxBadge />);

    const badge = screen.getByText("No sandbox");
    expect(badge.getAttribute("title")).toMatch(/reads, writes and network/);
    expect(badge.getAttribute("title")).toMatch(/logins and secrets/);
  });

  it("shows nothing before the PM's sandbox has been looked at", () => {
    const { container } = render(<NoSandboxBadge />);

    expect(container.textContent).toBe("");
  });

  it("shows nothing for a session that runs in its sandbox", () => {
    useChat.setState({ sandbox: { refusal: null, unsandboxed: false } });
    const { container } = render(<NoSandboxBadge />);

    expect(container.textContent).toBe("");
  });

  it("shows nothing for a refusal the project has not allowed running past", () => {
    useChat.setState({
      sandbox: { refusal: "no_namespaces", unsandboxed: false },
    });
    const { container } = render(<NoSandboxBadge />);

    expect(container.textContent).toBe("");
  });
});
