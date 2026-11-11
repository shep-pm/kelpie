import { useCallback, useEffect, useState } from "react";
import { commands, type GithubStatus } from "@/bindings";
import { Button } from "@/components/ui/button";

/** What the row says for each answer `gh` gives */
function said(status: GithubStatus) {
  switch (status.kind) {
    case "signed_in":
      return (
        <>
          signed in as <span className="font-mono">{status.login}</span>
        </>
      );
    case "not_signed_in":
      return (
        <>
          not signed in. Run <code className="font-mono">gh auth login</code> in
          a terminal, then Refresh
        </>
      );
    case "no_gh":
      return (
        <>
          <code className="font-mono">gh</code> is not installed
        </>
      );
  }
}

/**
 * The GitHub row: whose `gh` login every read and every draft pull request goes
 * out as (`docs/design/tickets.md`, GitHub). Kelpie holds no GitHub token and
 * runs no sign-in, so the row only asks `gh` who it holds and never shows a
 * token. A maintainer without a login runs `gh auth login` in a terminal and
 * presses Refresh.
 */
export function GithubField() {
  // `null` until `gh` has answered who it holds
  const [status, setStatus] = useState<GithubStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  const read = useCallback(async () => {
    const seen = await commands.githubStatus();
    if (seen.status === "error") {
      setError(seen.error);
      return;
    }
    setError(null);
    setStatus(seen.data);
  }, []);

  useEffect(() => {
    void read();
  }, [read]);

  return (
    <fieldset
      aria-labelledby="github-label"
      className="grid grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)] gap-x-6 gap-y-1 border-b py-3"
    >
      <div className="flex flex-col gap-0.5">
        <span id="github-label" className="text-label font-medium">
          GitHub
        </span>
        <p className="text-xs text-muted-foreground">
          Kelpie holds no GitHub token. Every read and draft pull request goes
          out as this login.
        </p>
      </div>
      <div className="flex min-w-0 flex-col gap-1">
        <div className="flex flex-wrap items-center gap-2 px-0.5">
          {status !== null && <span className="text-sm">{said(status)}</span>}
          <Button
            type="button"
            variant="outline"
            size="xs"
            onClick={() => void read()}
          >
            Refresh
          </Button>
        </div>
        {status?.kind === "not_signed_in" && (
          <p className="px-0.5 text-xs text-muted-foreground">{status.line}</p>
        )}
        {error !== null && (
          <p role="alert" className="px-0.5 text-xs text-status-failed-fg">
            {error}
          </p>
        )}
      </div>
    </fieldset>
  );
}
