import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { SandboxSetup } from "./SandboxSetup";

describe("the sandbox setup item", () => {
  afterEach(cleanup);

  it("no_capabilities says Ubuntu restricts the sandbox", () => {
    render(<SandboxSetup refusal="no_capabilities" />);

    expect(screen.getByText("Ubuntu restricts the sandbox")).toBeTruthy();
    expect(
      screen.getByText(
        "Ubuntu 24.04 and later stop the sandbox from starting until kelpie's AppArmor profile is installed",
      ),
    ).toBeTruthy();
  });

  it("no_namespaces says this kernel has no sandbox", () => {
    render(<SandboxSetup refusal="no_namespaces" />);

    expect(screen.getByText("This kernel has no sandbox")).toBeTruthy();
    expect(
      screen.getByText(
        "The kernel gives no program a user namespace, and no profile can change that",
      ),
    ).toBeTruthy();
  });

  it("has no button yet, since the install and the opt-out come later", () => {
    render(<SandboxSetup refusal="no_capabilities" />);

    expect(screen.queryByRole("button")).toBeNull();
  });
});
