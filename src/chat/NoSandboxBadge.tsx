import { Badge } from "@/components/ui/badge";
import { useChat } from "./store";

const WHY =
  "The PM runs without a sandbox: its reads, writes and network are your own, logins and secrets included. Kelpie's tools and the PM's tool list still apply.";

/**
 * The red mark beside the PM's title while its session runs unsandboxed
 * (`docs/design/sandbox-setup.md`, Running without a sandbox).
 *
 * It reads what the store last asked the PM's sandbox, so it shows from the end
 * of the first send until the session starts over in a sandbox. Failed status
 * tokens and the failed glyph, since the person should not miss it
 * (`docs/design/system/`, StatusBadge).
 */
export function NoSandboxBadge() {
  const unsandboxed = useChat((s) => s.sandbox?.unsandboxed ?? false);
  if (!unsandboxed) return null;
  return (
    <Badge
      title={WHY}
      className="h-5 gap-1 rounded-sm bg-status-failed-soft px-1.5 text-status-failed-fg"
    >
      <svg
        viewBox="0 0 12 12"
        className="size-3 shrink-0 text-status-failed"
        aria-hidden="true"
      >
        <path
          d="M3 3 9 9M9 3 3 9"
          stroke="currentColor"
          strokeWidth="1.75"
          strokeLinecap="round"
          fill="none"
        />
      </svg>
      No sandbox
    </Badge>
  );
}
