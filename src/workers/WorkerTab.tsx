import { useEffect, useState } from "react";
import { commands } from "@/bindings";
import { useBoard } from "@/board/store";
import { ChatBox } from "@/chat/ChatBox";
import { ChatProvider } from "@/chat/context";
import { type ChatStore, createChat } from "@/chat/store";
import { Button } from "@/components/ui/button";
import { workerTransport } from "./transport";

/** The ticket a worker is on: its number and its title. */
export type WorkerTicket = { id: string; title: string };

/**
 * Whether the board marks the worker on `id` as Paused. The engine words the
 * mark `Paused, resumes when you say so` (`src-tauri/src/board/mod.rs`), and the
 * board is the only place the window learns a worker's state.
 */
function usePaused(id: string) {
  return useBoard(
    (s) =>
      s.view?.tickets
        .find((t) => t.id === id)
        ?.needs_you.some((line) => line.startsWith("Paused")) ?? false,
  );
}

function Header({ ticket }: { ticket: WorkerTicket }) {
  const paused = usePaused(ticket.id);
  const [resumed, setResumed] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);

  async function resume() {
    setProblem(null);
    const result = await commands.workerResume(ticket.id);
    if (result.status === "error") return setProblem(result.error);
    setResumed(true);
    void commands.boardRefresh();
  }

  return (
    <header className="flex shrink-0 flex-col gap-1 border-b px-4 py-2">
      <div className="flex items-center gap-2">
        <h2 className="min-w-0 flex-1 truncate text-base font-semibold">
          <span className="font-mono text-muted-foreground">#{ticket.id}</span>{" "}
          {ticket.title}
        </h2>
        {paused && !resumed && (
          <Button type="button" size="sm" onClick={() => void resume()}>
            Resume
          </Button>
        )}
      </div>
      {problem && (
        <p role="alert" className="text-sm text-status-failed-fg">
          {problem}
        </p>
      )}
    </header>
  );
}

/**
 * A worker's tab in the PM panel: the PM's chat view over the worker's session,
 * read-only until a worker takes messages (#153).
 *
 * The chat is made once the tab mounts and closed when it unmounts, so a
 * closed tab stops following the worker, which keeps running. Opening the tab
 * again reads the history afresh.
 */
export function WorkerTab({ ticket }: { ticket: WorkerTicket }) {
  const [store, setStore] = useState<ChatStore | null>(null);

  useEffect(() => {
    const chat = createChat(workerTransport(ticket.id));
    setStore(() => chat); // a store is a function, which setState would call
    return () => chat.getState().close();
  }, [ticket.id]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <Header ticket={ticket} />
      {store && (
        <ChatProvider store={store}>
          <ChatBox picker={false} readOnly />
        </ChatProvider>
      )}
    </div>
  );
}
