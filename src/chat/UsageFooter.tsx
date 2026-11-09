import type { CacheLifetime, Usage } from "@/bindings";

const LIFETIME: Record<CacheLifetime, string> = {
  five_minutes: "5m",
  one_hour: "1h",
};

/** 1234 -> "1.2k", 2_500_000 -> "2.5M", to one decimal, as the footer's rules say. */
function abbreviate(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

/**
 * One line of what a turn cost. A cache field of `0` and a missing lifetime
 * mean the harness did not say, so they are left out rather than shown as 0
 * (ADR 0007).
 */
export function UsageFooter({ usage }: { usage: Usage }) {
  const fields = [
    `${abbreviate(usage.input)} in`,
    `${abbreviate(usage.output)} out`,
    usage.cache_read > 0 && `cache read ${abbreviate(usage.cache_read)}`,
    usage.cache_write > 0 && `cache written ${abbreviate(usage.cache_write)}`,
    usage.cache_lifetime && `cache ${LIFETIME[usage.cache_lifetime]}`,
  ].filter((field): field is string => field !== false && field !== null);

  return (
    <p
      role="note"
      aria-label="Usage"
      className="font-mono text-xs text-muted-foreground tabular-nums"
    >
      {fields.map((field, i) => (
        <span key={field}>
          {i > 0 && <span className="opacity-50">{" · "}</span>}
          {field}
        </span>
      ))}
    </p>
  );
}
