import type { RefusalView } from "@/bindings";
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
 * Why the PM's sandbox cannot start, in the turn that found out.
 *
 * It stands in for the raw error line while the sandbox says it refused. It has
 * no button yet: Install and Run without a sandbox come with their own slices.
 */
export function SandboxSetup({ refusal }: { refusal: RefusalView }) {
  const { title, detail } = WORDS[refusal];
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
    </div>
  );
}
