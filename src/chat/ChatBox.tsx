import { type KeyboardEvent, useEffect, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { commands } from "@/bindings";
import { useBoard } from "@/board/store";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Textarea } from "@/components/ui/textarea";
import { useChatStore } from "./context";
import { Turn } from "./Turn";

/** Keeps the newest text in view as a reply streams in. */
function ScrollAnchor() {
  const useChat = useChatStore();
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
  const useChat = useChatStore();
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

/** What one session's composer takes. */
type ComposerProps = {
  /** The composer of a session the maintainer tells while it runs. */
  steer?: boolean;
  /** Who the box is talking to, which its placeholder names. */
  to?: string;
};

/**
 * The message box, and the buttons the session in front of it takes.
 *
 * The PM's composer sends, and stops a turn. A steered session is told while it
 * runs: Note is the box's own send, Stop and tell interrupts the turn and sends
 * what is typed as the next one, and Stop cancels the turn and leaves the session
 * for the maintainer to resume (`docs/design/system/components/Composer`).
 */
function Composer({ steer = false, to = "the PM" }: ComposerProps) {
  const useChat = useChatStore();
  const [text, setText] = useState("");
  const running = useChat((s) => s.turns.at(-1)?.state === "running");
  // An install ends the PM's session, so a message sent meanwhile would be cut off
  const installing = useChat((s) => s.installing);

  function typed() {
    const message = text.trim();
    if (message === "") return null;
    setText("");
    return message;
  }

  function send() {
    if (running || installing) return;
    const message = typed();
    if (message === null) return;
    void useChat.getState().send(message);
  }

  function note() {
    const message = typed();
    if (message === null) return;
    void useChat.getState().note(message);
  }

  function stopAndTell() {
    const message = typed();
    if (message === null) return;
    void useChat.getState().stopAndTell(message);
  }

  function stop() {
    void useChat.getState().cancel();
  }

  function enter(e: KeyboardEvent) {
    if (e.key !== "Enter" || e.shiftKey || e.nativeEvent.isComposing) return;
    e.preventDefault();
    if (!steer) return send();
    // Enter is the note, and the chord is the interrupt (`docs/design/system/components/Composer`)
    if (e.metaKey || e.ctrlKey) stopAndTell();
    else note();
  }

  return (
    <div className="mx-auto flex w-full max-w-3xl items-end gap-2 px-4 pb-4">
      <Textarea
        aria-label="Message"
        placeholder={`Message ${to}`}
        value={text}
        rows={1}
        className="max-h-48 flex-1 resize-none"
        onChange={(e) => setText(e.target.value)}
        onKeyDown={enter}
      />
      {steer ? (
        <>
          {running && (
            <Button variant="outline" onClick={stop}>
              Stop
            </Button>
          )}
          {running && (
            <Button variant="destructive" onClick={stopAndTell}>
              Stop and tell
            </Button>
          )}
          <Button onClick={note} disabled={text.trim() === ""}>
            Note
          </Button>
        </>
      ) : running ? (
        <Button variant="destructive" onClick={stop}>
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
    else if (result.data) {
      onPicked(result.data);
      // The board watched before any project existed and was refused.
      useBoard.getState().rewatch();
    }
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

/**
 * One chat's front end: the turns, and the composer under them.
 *
 * `picker` shows the project picker until a project is kept, which only the
 * PM's tab needs. A chat without it reads its history at once.
 *
 * `steer` is the composer of a session the maintainer tells while it runs, which
 * is a worker's. A chat with neither sends between turns and stops a turn.
 *
 * `to` is who the box talks to, which its placeholder names ("Message the PM",
 * "Message the worker on #42", `docs/design/system/components/Composer`).
 */
export function ChatBox({
  picker = true,
  steer = false,
  to = "the PM",
}: {
  picker?: boolean;
  steer?: boolean;
  to?: string;
}) {
  const useChat = useChatStore();
  // `undefined` until the backend says whether a project is kept.
  const [project, setProject] = useState<string | null | undefined>(
    picker ? undefined : "",
  );

  useEffect(() => {
    if (!picker) return;
    let live = true;
    commands
      .projectGet()
      .then((folder) => live && setProject(folder))
      .catch(() => live && setProject(null));
    return () => {
      live = false;
    };
  }, [picker]);

  // History is read once the project is known, before anything is sent.
  useEffect(() => {
    if (project !== undefined && project !== null) {
      void useChat.getState().load();
    }
  }, [project, useChat]);

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
      <Composer steer={steer} to={to} />
    </main>
  );
}
