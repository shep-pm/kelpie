import { useEffect, useState } from "react";
import { ChatBox } from "@/chat/ChatBox";
import { ChatProvider } from "@/chat/context";
import { type ChatStore, createChat } from "@/chat/store";
import { reviewerTransport } from "./transport";

/** The read a tab shows: the ticket it reads, and who is reading it. */
export type ReviewTicket = {
  id: string;
  title: string;
  /** The reviewer, as the project lists it. `null` when kelpie has seen no read of it. */
  reviewer: string | null;
};

function Header({ ticket }: { ticket: ReviewTicket }) {
  return (
    <header className="flex shrink-0 items-center gap-2 border-b px-4 py-2">
      <h2 className="min-w-0 flex-1 truncate text-base font-semibold">
        <span className="font-mono text-muted-foreground">#{ticket.id}</span>{" "}
        {ticket.title}
      </h2>
      <span className="shrink-0 text-sm text-muted-foreground">
        {ticket.reviewer ?? "Reviewer"}
      </span>
    </header>
  );
}

/**
 * A read's tab in the PM panel, beside the worker's: the PM's chat view over the
 * reviewer's session, read-only, with each look at the pull request behind the line naming
 * it (`docs/design/review.md`, What the maintainer sees).
 *
 * The chat is made once the tab mounts and closed when it unmounts, so a closed tab stops
 * following the read. The read is the pass's own, and it goes on without the tab. What a
 * read found reaches the worker as the worker's next message, so the worker's tab is where
 * the findings are read.
 */
export function ReviewerTab({ ticket }: { ticket: ReviewTicket }) {
  const [store, setStore] = useState<ChatStore | null>(null);

  useEffect(() => {
    const chat = createChat(reviewerTransport(ticket.id));
    setStore(() => chat); // a store is a function, which setState would call
    return () => chat.getState().close();
  }, [ticket.id]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <Header ticket={ticket} />
      {store && (
        <ChatProvider store={store} who="Reviewer" readOnly>
          <ChatBox picker={false} readOnly />
        </ChatProvider>
      )}
    </div>
  );
}
