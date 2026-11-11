import { useState } from "react";
import {
  commands,
  type Install,
  type InstallOutcome,
  type RefusalView,
} from "@/bindings";
import { Button } from "@/components/ui/button";
import { useChatStore } from "./context";
import { NeedsYou } from "./SandboxRequest";

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

/** What an install that did not finish leaves in the item. */
type Left = Exclude<InstallOutcome, { kind: "installed" }>;

/**
 * Why the PM's sandbox cannot start, in the turn that found out.
 *
 * It stands in for the raw error line while the sandbox says it refused.
 * *Install* loads kelpie's AppArmor profile through one password prompt, then
 * sends the failed message again, and is offered where the package put the
 * profile (`install: "available"`) and the profile can fix the refusal. An
 * AppImage or a dev build is offered only *Run without a sandbox*, and the item
 * says the `.deb` or `.rpm` can install the profile. *Run without a sandbox*
 * opts the project in and sends the failed message again, so the PM starts
 * without it being typed twice. While the PM runs unsandboxed the item stays,
 * with a muted line saying so and Install, if it is offered, in place of the
 * button that got it there.
 */
export function SandboxSetup({
  refusal,
  unsandboxed = false,
  install = "package_only",
}: {
  refusal: RefusalView;
  /** The PM runs without a sandbox now. */
  unsandboxed?: boolean;
  /** Whether Install can load the profile on this machine. */
  install?: Install;
}) {
  const { title, detail } = WORDS[refusal];
  const useChat = useChatStore();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [left, setLeft] = useState<Left | null>(null);
  // Installing ends the session, which would cut a turn in flight off
  const turnRuns = useChat((s) => s.turns.some((t) => t.state === "running"));
  const profileFixes = refusal === "no_capabilities";
  const installable = profileFixes && install === "available";

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

  async function installProfile() {
    setBusy(true);
    setError(null);
    setLeft(null);
    const outcome = await useChat.getState().install();
    if (outcome.kind !== "installed") setLeft(outcome);
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
      {unsandboxed && (
        <p className="text-label font-normal text-muted-foreground">
          The PM runs without a sandbox on this project.
        </p>
      )}
      {profileFixes && install === "package_only" && (
        <p className="text-label font-normal text-muted-foreground">
          The .deb or .rpm can install the profile.
        </p>
      )}
      {(installable || !unsandboxed) && (
        <div className="flex flex-wrap gap-1.5">
          {installable && (
            <Button
              size="sm"
              disabled={busy || turnRuns}
              onClick={() => void installProfile()}
            >
              Install
            </Button>
          )}
          {!unsandboxed && (
            <Button
              size="sm"
              variant="secondary"
              disabled={busy}
              onClick={() => void runWithout()}
            >
              Run without a sandbox
            </Button>
          )}
        </div>
      )}
      {left?.kind === "dismissed" && (
        <p className="text-label font-normal text-muted-foreground">
          The password prompt was dismissed, so nothing was installed.
        </p>
      )}
      {left?.kind === "no_agent" && (
        <div className="flex flex-col gap-1">
          <p className="text-label font-normal text-muted-foreground">
            No password prompt could open. Run these in a terminal, then send
            the message again:
          </p>
          {left.commands.map((command) => (
            <code
              key={command}
              className="rounded-md bg-muted px-2 py-1 font-mono text-label"
            >
              {command}
            </code>
          ))}
        </div>
      )}
      {left?.kind === "failed" && (
        <p
          role="alert"
          className="text-label font-normal text-status-failed-fg"
        >
          {left.message}
        </p>
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
