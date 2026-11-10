import { cn } from "@/lib/utils";
import type { Step } from "./store";

type Line = { key: string; text: string; failed: boolean };

/** The kind of a step is the first word of its title: "Read", "Grep", "Bash". */
function kindOf(step: Step): string {
  return step.title.trim().split(/\s+/)[0] ?? "";
}

const PLURAL: Record<string, (n: number) => string> = {
  Read: (n) => `Read ${n} files`,
  Edit: (n) => `Edited ${n} files`,
  Write: (n) => `Wrote ${n} files`,
  Grep: (n) => `Searched ${n} times`,
  Glob: (n) => `Searched ${n} times`,
  Search: (n) => `Searched ${n} times`,
};

function merged(kind: string, count: number): string {
  return PLURAL[kind]?.(count) ?? `${kind} · ${count} steps`;
}

/** Consecutive steps of one kind become one line; a lone step keeps its title. */
export function fold(steps: Step[]): Line[] {
  const lines: Line[] = [];
  let i = 0;
  while (i < steps.length) {
    const first = steps[i] as Step;
    const kind = kindOf(first);
    let end = i + 1;
    while (end < steps.length && kindOf(steps[end] as Step) === kind) end++;
    const count = end - i;
    lines.push({
      key: first.id,
      text: count === 1 ? first.title : merged(kind, count),
      failed: steps
        .slice(i, end)
        .some((s) => s.status === "failed" || s.status === "refused"),
    });
    i = end;
  }
  return lines;
}

/** The turn's tool calls, one muted line each, so the reply stays the loudest thing. */
export function ToolSteps({ steps }: { steps: Step[] }) {
  if (steps.length === 0) return null;
  return (
    <ul
      aria-label="Steps"
      className="flex flex-col text-sm text-muted-foreground"
    >
      {fold(steps).map((line) => (
        <li
          key={line.key}
          className={cn("truncate", line.failed && "text-status-failed-fg")}
        >
          {line.text}
        </li>
      ))}
    </ul>
  );
}
