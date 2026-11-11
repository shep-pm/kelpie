import { useEffect, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { commands } from "@/bindings";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Textarea } from "@/components/ui/textarea";
import { useChat } from "./store";
import { Turn } from "./Turn";

/** Keeps the newest text in view as a reply streams in. */
function ScrollAnchor() {
  const length = useChat((s) => {
    const last = s.turns.at(-1);
    return (
      s.turns.length + (last?.reply.length ?? 0) + (last?.steps.length ?? 0)
    );
  });
  const ref = useRef<HTMLDivElement>(null);
  // biome-ignore lint/correctness/useExhaustiveDependencies: the length is the trigger.
  useEffect(() => {
    ref.current?.scrollIntoView?.({ block: "end" });
  }, [length]);
  return <div ref={ref} />;
}

function Turns() {
  const ids = useChat(useShallow((s) => s.turns.map((t) => t.id)));
  const historyError = useChat((s) => s.historyError);
  return (
    <ScrollArea className="min-h-0 flex-1">
      <div className="mx-auto flex max-w-3xl flex-col gap-6 px-4 py-4">
        {historyError && (
          <p
            role="alert"
            className="rounded-md bg-status-failed-soft px-3 py-1.5 text-sm text-status-failed-fg"
          >
            {historyError}
          </p>
        )}
        {ids.map((id) => (
          <Turn key={id} id={id} />
        ))}
        <ScrollAnchor />
      </div>
    </ScrollArea>
  );
}

function Composer() {
  const [text, setText] = useState("");
  const running = useChat((s) => s.turns.at(-1)?.state === "running");
  // An install ends the PM's session, so a message sent meanwhile would be cut off
  const installing = useChat((s) => s.installing);

  function send() {
    const message = text.trim();
    if (message === "" || running || installing) return;
    setText("");
    void useChat.getState().send(message);
  }

  return (
    <div className="mx-auto flex w-full max-w-3xl items-end gap-2 px-4 pb-4">
      <Textarea
        aria-label="Message"
        placeholder="Message the PM"
        value={text}
        rows={1}
        className="max-h-48 flex-1 resize-none"
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key !== "Enter" || e.shiftKey || e.nativeEvent.isComposing) {
            return;
          }
          e.preventDefault();
          send();
        }}
      />
      {running ? (
        <Button
          variant="destructive"
          onClick={() => void useChat.getState().cancel()}
        >
          Stop
        </Button>
      ) : (
        <Button onClick={send} disabled={text.trim() === "" || installing}>
          Send
        </Button>
      )}
    </div>
  );
}

function PickFolder({ onPicked }: { onPicked: (folder: string) => void }) {
  const [error, setError] = useState<string | null>(null);

  async function pick() {
    const result = await commands.projectPick();
    if (result.status === "error") setError(result.error);
    else if (result.data) onPicked(result.data);
  }

  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-3 p-4">
      <p className="text-base text-muted-foreground">
        Pick the folder of the project the PM works on.
      </p>
      <Button onClick={() => void pick()}>Pick a folder</Button>
      {error && (
        <p role="alert" className="text-sm text-status-failed-fg">
          {error}
        </p>
      )}
    </div>
  );
}

/** The PM's front end: the turns, and the composer under them. */
export function ChatBox() {
  // `undefined` until the backend says whether a project is kept.
  const [project, setProject] = useState<string | null | undefined>(undefined);

  useEffect(() => {
    let live = true;
    commands
      .projectGet()
      .then((folder) => live && setProject(folder))
      .catch(() => live && setProject(null));
    return () => {
      live = false;
    };
  }, []);

  // History is read once the project is known, before anything is sent.
  useEffect(() => {
    if (project) void useChat.getState().load();
  }, [project]);

  if (project === undefined) return <main className="min-h-0 flex-1" />;
  if (project === null) {
    return (
      <main className="flex min-h-0 flex-1 flex-col">
        <PickFolder onPicked={setProject} />
      </main>
    );
  }
  return (
    <main className="flex min-h-0 flex-1 flex-col">
      <Turns />
      <Composer />
    </main>
  );
}
