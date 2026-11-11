import { useState } from "react";
import { commands, type RefusalView } from "@/bindings";
import { Button } from "@/components/ui/button";
import { NeedsYou } from "./SandboxRequest";
import { useChat } from "./store";

/** What the maintainer reads for each way a sandbox can refuse to start (`docs/design/sandbox-setup.md`, Flow). */
const WORDS: Record<RefusalView, { title: string; detail: string }> = {
  no_capabilities: {
    title: "Ubuntu restricts the sandbox",
    detail:
      "Ubuntu 24.04 and later stop the sandbox from starting until kelpie's AppArmor profile is installed",
  },
  no_namespaces: {
    title: "This kernel has no sandbox",
    detail:
      "The kernel gives no program a user namespace, and no profile can change that",
  },
};

/**
 * Turn the project's `run-without-sandbox` on, in the project layer. What went
 * wrong, or `null` once it is written.
 */
async function allowRunningWithout(): Promise<string | null> {
  try {
    const view = await commands.settingsGet(null);
    if (view.status === "error") return view.error;
    const project = view.data.project;
    if (!project) return "There is no project to turn this on for";
    const done = await commands.settingsSet(
      { kind: "project", id: project.id },
      "run-without-sandbox",
      true,
    );
    return done.status === "error" ? done.error : null;
  } catch (error) {
    return String(error);
  }
}

/**
 * Why the PM's sandbox cannot start, in the turn that found out.
 *
 * It stands in for the raw error line while the sandbox says it refused.
 * *Run without a sandbox* opts the project in and sends the failed message
 * again, so the PM starts without it being typed twice. While the PM runs
 * unsandboxed the item stays, with a muted line saying so and no button: the
 * Install button comes with its own slice and belongs on it.
 */
export function SandboxSetup({
  refusal,
  unsandboxed = false,
}: {
  refusal: RefusalView;
  /** The PM runs without a sandbox now. */
  unsandboxed?: boolean;
}) {
  const { title, detail } = WORDS[refusal];
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function runWithout() {
    setBusy(true);
    setError(null);
    const failed = await allowRunningWithout();
    if (failed === null) {
      await useChat.getState().resend();
    } else {
      setError(failed);
    }
    setBusy(false);
  }

  return (
    <div
      role="alert"
      className="flex flex-col gap-2 rounded-lg bg-status-needs-soft p-3 text-sm"
    >
      <div className="flex items-center gap-2">
        <NeedsYou />
        <span className="font-semibold">{title}</span>
      </div>
      <p className="text-muted-foreground">{detail}</p>
      {unsandboxed ? (
        <p className="text-label font-normal text-muted-foreground">
          The PM runs without a sandbox on this project.
        </p>
      ) : (
        <div className="flex flex-wrap gap-1.5">
          <Button
            size="sm"
            variant="secondary"
            disabled={busy}
            onClick={() => void runWithout()}
          >
            Run without a sandbox
          </Button>
        </div>
      )}
      {error && (
        <p
          role="alert"
          className="text-label font-normal text-status-failed-fg"
        >
          {error}
        </p>
      )}
    </div>
  );
}
