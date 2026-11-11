import type { ComponentProps, ReactNode } from "react";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

/** The six states a ticket, worker, thread or process can be in. */
export type Status =
  | "idle"
  | "running"
  | "needs"
  | "blocked"
  | "done"
  | "failed";

/** The words the design system says to use exactly. */
export const STATUS_WORDS: Record<Status, string> = {
  idle: "Idle",
  running: "Running",
  needs: "Needs you",
  blocked: "Blocked",
  done: "Done",
  failed: "Failed",
};

/** Soft fill and text, both from the status's own tokens. */
const FILL: Record<Status, string> = {
  idle: "bg-status-idle-soft text-status-idle-fg",
  running: "bg-status-running-soft text-status-running-fg",
  needs: "bg-status-needs-soft text-status-needs-fg",
  blocked: "bg-status-blocked-soft text-status-blocked-fg",
  done: "bg-status-done-soft text-status-done-fg",
  failed: "bg-status-failed-soft text-status-failed-fg",
};

/** The glyph's own colour: the stronger one of the status's three tokens. */
const GLYPH_FILL: Record<Status, string> = {
  idle: "text-status-idle",
  running: "text-status-running",
  needs: "text-status-needs",
  blocked: "text-status-blocked",
  done: "text-status-done",
  failed: "text-status-failed",
};

/**
 * Hollow ring, spinning arc, filled diamond, ring with a bar, check and cross,
 * each 12px (`docs/design/system/components/StatusBadge/preview.html`).
 * Running's arc stops under reduced motion, which `src/styles/globals.css`
 * applies to every animation, and stays a still quarter arc.
 */
const GLYPHS: Record<Status, ReactNode> = {
  idle: (
    <circle
      cx="6"
      cy="6"
      r="4.25"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
    />
  ),
  running: (
    <>
      <circle
        cx="6"
        cy="6"
        r="4.25"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        opacity="0.28"
      />
      <path
        d="M6 1.75a4.25 4.25 0 0 1 4.25 4.25"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
        className="origin-center animate-spin"
      />
    </>
  ),
  needs: <path d="M6 1.2 10.8 6 6 10.8 1.2 6Z" fill="currentColor" />,
  blocked: (
    <>
      <circle
        cx="6"
        cy="6"
        r="4.25"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
      />
      <path
        d="M3.6 6h4.8"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
      />
    </>
  ),
  done: (
    <path
      d="M2.4 6.3 4.9 8.7 9.6 3.6"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.75"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  ),
  failed: (
    <path
      d="M3 3 9 9M9 3 3 9"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.75"
      strokeLinecap="round"
    />
  ),
};

/** The glyph every status carries, so the state reads without colour. */
export function StatusGlyph({
  status,
  className,
}: {
  status: Status;
  className?: string;
}) {
  return (
    <svg
      viewBox="0 0 12 12"
      data-slot="status-glyph"
      data-status={status}
      aria-hidden="true"
      className={cn("size-3 shrink-0", GLYPH_FILL[status], className)}
    >
      {GLYPHS[status]}
    </svg>
  );
}

type Props = {
  status: Status;
  /** A shorter or more specific word than the status's own, such as "Paused". */
  children?: ReactNode;
} & Omit<ComponentProps<"span">, "children">;

/**
 * One of the six states, as a word, a glyph and a colour
 * (`docs/design/system/`, StatusBadge), on shadcn's `Badge`.
 *
 * The colour and glyph come from `status`; a caller never passes either. A
 * paused worker is `needs` with the word "Paused": its process stopped mid-turn
 * and it resumes only when the maintainer says so.
 *
 * Needs you is the only status ever shown solid, and that is `NeedsYouCount`,
 * not this badge.
 */
export function StatusBadge({ status, children, className, ...props }: Props) {
  return (
    <Badge
      data-slot="status-badge"
      data-status={status}
      className={cn(
        "h-5 gap-1 rounded-sm px-1.5 py-0",
        FILL[status],
        className,
      )}
      {...props}
    >
      <StatusGlyph status={status} />
      {children ?? STATUS_WORDS[status]}
    </Badge>
  );
}
