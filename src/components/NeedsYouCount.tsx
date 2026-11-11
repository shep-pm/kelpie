import { cn } from "@/lib/utils";

/**
 * What a count says out loud, for the label of the tab or button it sits on.
 * The count itself is only a number, which a reader cannot place.
 */
export function needsYouLabel(count: number) {
  return count === 1 ? "1 needs you" : `${count} need you`;
}

/**
 * One count of what waits on the maintainer, wherever it sits
 * (`docs/design/system/`, NeedsYouCount, and `docs/design/ux.md`, Attention).
 *
 * Solid `status-needs` with `status-needs-on` text: it is the only solid status
 * in the app, so it is the first thing the eye finds. Keep it that way by never
 * using the tan for anything else.
 *
 * It is never a target of its own. The tile, tab or item it sits on is what a
 * click answers.
 */
export function NeedsYouCount({
  count,
  className,
}: {
  /** How many waits this holds. Nothing shows at 0: an empty badge is noise. */
  count: number;
  /** Where the count sits on what carries it, such as a button's corner. */
  className?: string;
}) {
  if (count <= 0) return null;
  return (
    <span
      data-slot="needs-you-count"
      className={cn(
        "inline-flex h-4 min-w-4 shrink-0 items-center justify-center rounded-full bg-status-needs px-1 text-xs font-bold text-status-needs-on tabular-nums",
        className,
      )}
    >
      {count > 99 ? "99+" : count}
    </span>
  );
}
