import { RotateCcw, X } from "lucide-react";
import { useState } from "react";
import { Streamdown } from "streamdown";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleTrigger } from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";
import { type Turn as TurnData, useChat } from "./store";
import { ToolSteps } from "./ToolSteps";
import { UsageFooter } from "./UsageFooter";

/** A reply longer than this many lines (22px each, a screenful) folds behind "Show the rest". */
export const SCREENFUL = 24;

/** The turn with `id`, read one field at a time so a delta re-renders only its reader. */
function useField<K extends keyof TurnData>(id: number, key: K) {
  return useChat((s) => s.turns.find((t) => t.id === id)?.[key]);
}

function UserBubble({ id }: { id: number }) {
  const user = useField(id, "user");
  return (
    <p className="ml-auto max-w-[85%] rounded-lg bg-secondary px-3 py-1.5 text-base whitespace-pre-wrap text-secondary-foreground">
      {user}
    </p>
  );
}

function StepsOfTurn({ id }: { id: number }) {
  const steps = useField(id, "steps");
  return <ToolSteps steps={steps ?? []} />;
}

function Reply({ id }: { id: number }) {
  const reply = useField(id, "reply") ?? "";
  const running = useField(id, "state") === "running";
  const [open, setOpen] = useState(false);

  if (reply === "") return null;
  // The reply stays one Markdown document, so a code block or a list that
  // crosses the fold is not cut in two. The fold clamps its height instead.
  const hidden = reply.split("\n").length - SCREENFUL;
  const folded = hidden > 0 && !open;

  return (
    <Collapsible open={open} onOpenChange={setOpen}>
      <div
        data-state={folded ? "closed" : "open"}
        className={cn(folded && "max-h-[528px] overflow-hidden")}
      >
        <Streamdown
          className="text-base [&_p]:my-0 [&>*+*]:mt-2"
          isAnimating={running}
        >
          {reply}
        </Streamdown>
      </div>
      {hidden > 0 && (
        <CollapsibleTrigger asChild>
          <Button
            variant="ghost"
            size="sm"
            className="mt-1 text-muted-foreground"
          >
            {open ? "Show less" : `Show the rest · ${hidden} lines`}
          </Button>
        </CollapsibleTrigger>
      )}
    </Collapsible>
  );
}

function Failure({ id }: { id: number }) {
  const error = useField(id, "error");
  if (!error) return null;
  return (
    <p
      role="alert"
      className="rounded-md bg-status-failed-soft px-3 py-1.5 text-sm text-status-failed-fg"
    >
      {error}
    </p>
  );
}

/**
 * What a quit cut off (`TurnMarkers`): a badge under the message, and Retry on
 * the last turn, the only one that can be sent again.
 */
function Interrupted({ id }: { id: number }) {
  const state = useField(id, "state");
  const last = useChat((s) => s.turns.at(-1)?.id === id);
  if (state !== "interrupted") return null;
  return (
    <div className="flex items-center justify-end gap-2">
      <span className="flex items-center gap-2 text-xs text-status-failed-fg">
        <X className="size-3" strokeWidth={1.75} aria-hidden="true" />
        Interrupted when kelpie quit
      </span>
      {last && (
        <Button
          variant="outline"
          size="sm"
          onClick={() => void useChat.getState().retry()}
        >
          <RotateCcw aria-hidden="true" />
          Retry
        </Button>
      )}
    </div>
  );
}

function Footer({ id }: { id: number }) {
  const usage = useField(id, "usage");
  return usage ? <UsageFooter usage={usage} /> : null;
}

export function Turn({ id }: { id: number }) {
  return (
    <article className="flex flex-col gap-2">
      <UserBubble id={id} />
      <Interrupted id={id} />
      <StepsOfTurn id={id} />
      <Reply id={id} />
      <Failure id={id} />
      <Footer id={id} />
    </article>
  );
}
