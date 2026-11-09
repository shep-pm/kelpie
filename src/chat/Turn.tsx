import { useState } from "react";
import { Streamdown } from "streamdown";
import { Button } from "@/components/ui/button";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { type Turn as TurnData, useChat } from "./store";
import { ToolSteps } from "./ToolSteps";
import { UsageFooter } from "./UsageFooter";

/** A reply longer than this many lines folds behind "Show the rest". */
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
  const lines = reply.split("\n");
  const hidden = lines.length - SCREENFUL;
  const prose = "text-base [&_p]:my-0 [&>*+*]:mt-2";

  if (hidden <= 0) {
    return (
      <Streamdown className={prose} isAnimating={running}>
        {reply}
      </Streamdown>
    );
  }

  return (
    <Collapsible open={open} onOpenChange={setOpen}>
      <Streamdown className={prose} isAnimating={running && !open}>
        {lines.slice(0, SCREENFUL).join("\n")}
      </Streamdown>
      <CollapsibleContent>
        <Streamdown className={prose} isAnimating={running}>
          {lines.slice(SCREENFUL).join("\n")}
        </Streamdown>
      </CollapsibleContent>
      <CollapsibleTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          className="mt-1 text-muted-foreground"
        >
          {open ? "Show less" : `Show the rest · ${hidden} lines`}
        </Button>
      </CollapsibleTrigger>
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

function Footer({ id }: { id: number }) {
  const usage = useField(id, "usage");
  return usage ? <UsageFooter usage={usage} /> : null;
}

export function Turn({ id }: { id: number }) {
  return (
    <article className="flex flex-col gap-2">
      <UserBubble id={id} />
      <StepsOfTurn id={id} />
      <Reply id={id} />
      <Failure id={id} />
      <Footer id={id} />
    </article>
  );
}
