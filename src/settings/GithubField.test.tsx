import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { GithubStatus } from "@/bindings";

const ok = <T,>(data: T) => ({ status: "ok" as const, data });

const githubStatus = vi.fn<() => Promise<unknown>>();

vi.mock("@/bindings", () => ({
  commands: { githubStatus: () => githubStatus() },
}));

import { GithubField } from "./GithubField";

afterEach(() => {
  cleanup();
  githubStatus.mockReset();
});

/** The row, with `gh` answering `first`, and `then` for every read after it. */
async function openRow(first: GithubStatus, then?: GithubStatus) {
  githubStatus.mockResolvedValueOnce(ok(first));
  if (then !== undefined) githubStatus.mockResolvedValue(ok(then));
  render(<GithubField />);
  return screen.findByRole("group", { name: "GitHub" });
}

describe("the GitHub row", () => {
  it("says which login kelpie acts as", async () => {
    const row = await openRow({ kind: "signed_in", login: "octocat" });

    await waitFor(() => expect(row.textContent).toContain("octocat"));
    expect(row.textContent).toContain("signed in as octocat");
  });

  it("says to run gh auth login when gh has no login", async () => {
    const line =
      "gh: To get started with GitHub CLI, please run:  gh auth login";
    const row = await openRow({ kind: "not_signed_in", line });

    await waitFor(() =>
      expect(row.textContent).toContain(
        "not signed in. Run gh auth login in a terminal, then Refresh",
      ),
    );
    // what gh itself said, so a failure that is not a missing login is visible
    expect(row.textContent).toContain(line);
  });

  it("says gh is not installed", async () => {
    const row = await openRow({ kind: "no_gh" });

    await waitFor(() =>
      expect(row.textContent).toContain("gh is not installed"),
    );
  });

  it("reads the status again on Refresh", async () => {
    const row = await openRow(
      { kind: "no_gh" },
      { kind: "signed_in", login: "octocat" },
    );
    await waitFor(() =>
      expect(row.textContent).toContain("gh is not installed"),
    );

    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));

    await waitFor(() =>
      expect(row.textContent).toContain("signed in as octocat"),
    );
    expect(githubStatus).toHaveBeenCalledTimes(2);
  });
});
