import { RotateCcw, X } from "lucide-react";
import { useState } from "react";
import { Streamdown } from "streamdown";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleTrigger } from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";
import { useChatStore, useWho } from "./context";
import { Divider } from "./Divider";
import { SandboxRequest } from "./SandboxRequest";
import { SandboxSetup } from "./SandboxSetup";
import type { Turn as TurnData } from "./store";
import { ToolSteps } from "./ToolSteps";
import { UsageFooter } from "./UsageFooter";

/** A reply longer than this many lines (22px each, a screenful) folds behind "Show the rest". */
export const SCREENFUL = 24;

/** The turn with `id`, read one field at a time so a delta re-renders only its reader. */
function useField<K extends keyof TurnData>(id: number, key: K) {
  const useChat = useChatStore();
  return useChat((s) => s.turns.find((t) => t.id === id)?.[key]);
}

function Dividers({ id }: { id: number }) {
  const dividers = useField(id, "dividers");
  // A retried turn can hold the same text twice: the repeat count keeps keys unique.
  const seen = new Map<string, number>();
  return dividers?.map((text) => {
    const nth = seen.get(text) ?? 0;
    seen.set(text, nth + 1);
    return <Divider key={`${text}#${nth}`} text={text} />;
  });
}

function UserBubble({ id }: { id: number }) {
  const user = useField(id, "user");
  // A turn a watched session began on its own has no message of the maintainer's
  if (!user) return null;
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

/**
 * The turn's sandbox requests, answered or waiting.
 *
 * Only the first waiting one is open: the rest queue behind it as a count, so a
 * turn never stacks prompts (`docs/design/system/`, SandboxRequest).
 */
function RequestsOfTurn({ id }: { id: number }) {
  const useChat = useChatStore();
  const who = useWho();
  const requests = useField(id, "sandboxRequests") ?? [];
  const open = requests.filter((asked) => asked.answer === null);
  const first = open[0];
  return requests.map((asked) =>
    asked.answer === null && asked !== first ? null : (
      <SandboxRequest
        key={asked.id}
        request={asked}
        queued={asked === first ? open.length - 1 : 0}
        who={who}
        onAnswer={(answer) => void useChat.getState().answer(asked.id, answer)}
      />
    ),
  );
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

/**
 * The Notes the maintainer sent while this turn ran, under it.
 *
 * A worker's own message is what is being read, so a note keeps the maintainer's
 * right-aligned bubble and adds the line saying what kelpie did with it.
 */
function NotesOfTurn({ id }: { id: number }) {
  const notes = useField(id, "notes") ?? [];
  // The same text can be noted twice: the repeat count keeps keys unique.
  const seen = new Map<string, number>();
  return notes.map((note) => {
    const nth = seen.get(note.text) ?? 0;
    seen.set(note.text, nth + 1);
    return (
      <div
        key={`${note.text}#${nth}`}
        className="flex flex-col items-end gap-1"
      >
        <p className="max-w-[85%] rounded-lg bg-secondary px-3 py-1.5 text-base whitespace-pre-wrap text-secondary-foreground">
          {note.text}
        </p>
        <p
          className={cn(
            "text-label font-normal",
            note.error ? "text-status-failed-fg" : "text-muted-foreground",
          )}
        >
          {note.error ?? note.line}
        </p>
      </div>
    );
  });
}

function Failure({ id }: { id: number }) {
  const useChat = useChatStore();
  const error = useField(id, "error");
  const state = useField(id, "state");
  const last = useChat((s) => s.turns.at(-1)?.id === id);
  const sandbox = useChat((s) => s.sandbox);
  const refusal = sandbox?.refusal ?? null;
  const unsandboxed = sandbox?.unsandboxed ?? false;
  // Only the last turn says it: an older failed turn kept its own error. A
  // session that runs unsandboxed keeps the item on its last turn, whatever
  // that turn did, since Install belongs on it.
  const item =
    refusal && last && (unsandboxed || (state === "failed" && !!error)) ? (
      <SandboxSetup
        refusal={refusal}
        unsandboxed={unsandboxed}
        install={sandbox?.install}
      />
    ) : null;
  // The item stands in for the raw error of the turn the sandbox refused. An
  // unsandboxed session runs past its refusal, so a failure in it is its own.
  const refused = item !== null && !unsandboxed;
  return (
    <>
      {error && !refused && (
        <p
          role="alert"
          className="rounded-md bg-status-failed-soft px-3 py-1.5 text-sm text-status-failed-fg"
        >
          {error}
        </p>
      )}
      {item}
    </>
  );
}

/**
 * What a quit cut off (`TurnMarkers`): a badge under the message, and Retry on
 * the last turn, the only one that can be sent again.
 */
function Interrupted({ id }: { id: number }) {
  const useChat = useChatStore();
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
      <Dividers id={id} />
      <UserBubble id={id} />
      <Interrupted id={id} />
      <StepsOfTurn id={id} />
      <RequestsOfTurn id={id} />
      <Reply id={id} />
      <NotesOfTurn id={id} />
      <Failure id={id} />
      <Footer id={id} />
    </article>
  );
}
