import { Globe } from "lucide-react";
import type { SandboxAnswer } from "@/bindings";
import { Button } from "@/components/ui/button";
import type { SandboxRequest as Asked } from "./store";

/** kelpie's own needs-you glyph: a filled diamond, 12px (`docs/design/system/`, StatusBadge). */
export function NeedsYou() {
  return (
    <svg
      viewBox="0 0 12 12"
      className="size-3 shrink-0 text-status-needs"
      role="img"
      aria-label="Needs you"
    >
      <path d="M6 1.2 10.8 6 6 10.8 1.2 6Z" fill="currentColor" />
    </svg>
  );
}

/** What the maintainer decided, in one muted line. */
const DECIDED: Record<SandboxAnswer, string> = {
  once: "allowed for this session",
  project: "allowed for this project",
  deny: "denied",
};

/**
 * One host the PM's sandbox refused, in the turn that asked for it.
 *
 * While it waits the maintainer answers in one click (`docs/design/system/`,
 * SandboxRequest): *Allow once* and *Allow for this project* both let the host
 * through for the rest of the session, and only *Allow for this project* is kept
 * to the project's settings, where it can be removed. Once answered it collapses
 * to one line, so the reply stays the loudest thing in the turn.
 *
 * `queued` is how many requests wait behind this one. Only the first is ever
 * open: the rest queue behind it as a count, so a turn never stacks prompts.
 */
export function SandboxRequest({
  request,
  queued = 0,
  onAnswer,
}: {
  request: Asked;
  /** Requests waiting behind this one. */
  queued?: number;
  onAnswer: (answer: SandboxAnswer) => void;
}) {
  if (request.error) {
    // The answer reached the sandbox but did not land where it was meant to,
    // so the line says what kelpie could not do instead of what it decided.
    return (
      <p
        role="alert"
        className="px-2 text-label font-normal text-status-failed-fg"
      >
        {request.error}
      </p>
    );
  }

  if (request.answer !== null) {
    const saved = request.answer === "project";
    return (
      <p className="flex items-center gap-2 px-2 text-label font-normal text-muted-foreground">
        <Globe
          className="size-3 shrink-0"
          strokeWidth={1.75}
          aria-hidden="true"
        />
        <span>
          {request.host} {DECIDED[request.answer]}
          {saved && (
            <>
              <span className="opacity-50">{" · "}</span>
              <span className="underline underline-offset-2">
                Undo in settings
              </span>
            </>
          )}
        </span>
      </p>
    );
  }

  return (
    <div className="flex flex-col gap-2 rounded-lg bg-status-needs-soft p-3 text-sm">
      <div className="flex items-center gap-2">
        <NeedsYou />
        <span className="font-semibold">PM wants to reach {request.host}</span>
        {queued > 0 && (
          <span className="text-xs font-medium text-muted-foreground">
            +{queued} more
          </span>
        )}
      </div>
      <p className="text-muted-foreground">{request.why}</p>
      <div className="flex flex-wrap gap-1.5">
        <Button size="sm" onClick={() => onAnswer("once")}>
          Allow once
        </Button>
        <Button
          size="sm"
          variant="secondary"
          onClick={() => onAnswer("project")}
        >
          Allow for this project
        </Button>
        <Button size="sm" variant="ghost" onClick={() => onAnswer("deny")}>
          Deny
        </Button>
      </div>
    </div>
  );
}
