import { useEffect, useState } from "react";
import { type Column, commands, type TicketView } from "@/bindings";
import { StatusBadge } from "@/components/StatusBadge";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Panel, type PanelControls } from "@/shell/Panel";
import {
  KanbanBoard,
  type KanbanColumn,
  type MoveOutcome,
} from "./KanbanBoard";
import { useBoard } from "./store";

/** The board's five columns, left to right. A ticket's column is a fact the engine reads, not a place it is put. */
const COLUMNS: readonly KanbanColumn<Column>[] = [
  { id: "draft", label: "Draft" },
  { id: "ready", label: "Ready" },
  { id: "building", label: "Building" },
  { id: "review", label: "Review" },
  { id: "done", label: "Done" },
];

const waits = (id: string) => `#${id} waits for a slot.`;

/**
 * The gate's merge yes, which Merge on the card or the Review to Done drag
 * answers, not a Note to the worker (`docs/design/gate-and-merge.md`, The Review
 * column). The engine words it `Ready to merge`
 * (`crates/kelpie-engine/src/gate/mod.rs`), and it is the one needs-you line
 * that is not the worker's own wait.
 */
const MERGE_YES = "Ready to merge";

/**
 * Whether the worker on `ticket` is the one waiting on the maintainer, which is
 * what a live worker's tab answers: a Paused worker, a held record with its
 * reason, or a sandbox request. A worker whose turn ended waits on nothing
 * (`crates/kelpie-engine/src/worker/mod.rs`, `Status::Ended`), so a Review
 * ticket whose only line is the merge yes has no tab worth opening.
 */
const workerWaits = (ticket: TicketView) =>
  ticket.needs_you.some((line) => line !== MERGE_YES);

/** The time a held limit lifts, in the maintainer's own clock. */
function Limit({ until }: { until: number }) {
  const at = new Date(until * 1000);
  return (
    <p
      data-slot="board-limit"
      className="shrink-0 px-1 pb-2 text-sm text-muted-foreground"
    >
      GitHub is limiting kelpie until{" "}
      <time dateTime={at.toISOString()}>
        {at.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}
      </time>
    </p>
  );
}

/**
 * The Board panel's body: the project's tickets in five columns, read from
 * GitHub and streamed by the engine.
 *
 * Ready tickets carry Start, and a drop from Ready on Building does the same.
 * Clicking a ticket whose worker is live calls `onOpenWorker`, as does a Start
 * that took a slot: a Building ticket, and a Review ticket whose worker waits
 * on the maintainer. Clicking any other opens its pull request in Review, else
 * its issue.
 */
export function Board({
  onOpenWorker,
}: {
  /** Open the worker on a ticket, which the PM panel's tabs answer. */
  onOpenWorker?: (ticket: { id: string; title: string }) => void;
}) {
  const view = useBoard((s) => s.view);
  const unavailable = useBoard((s) => s.unavailable);
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    useBoard.getState().watch();
    return () => useBoard.getState().close();
  }, []);

  async function start({ id, title }: TicketView) {
    setNotice(null);
    const result = await commands.workerStart(id);
    if (result.status === "error") return setNotice(result.error);
    if (result.data.waiting) return setNotice(waits(id));
    void commands.boardRefresh();
    // The tab opens where Start was pressed
    onOpenWorker?.({ id, title });
  }

  async function move(ticket: TicketView, to: Column): Promise<MoveOutcome> {
    setNotice(null);
    const result = await commands.boardMove(ticket.id, to);
    if (result.status === "error")
      return { kind: "snap_back", line: result.error };
    const moved = result.data;
    if (moved.kind === "snap_back") return moved;
    if (moved.kind === "started") {
      if (moved.waiting) return { kind: "snap_back", line: waits(ticket.id) };
      void commands.boardRefresh();
    }
    return { kind: "moved" };
  }

  async function open(ticket: TicketView) {
    if (ticket.column === "building")
      return onOpenWorker?.({ id: ticket.id, title: ticket.title });
    if (ticket.column === "review" && workerWaits(ticket))
      // The worker's own needs-you line asks the maintainer to answer that
      // worker, so the card opens the tab where they can, even after the tab
      // was closed.
      return onOpenWorker?.({ id: ticket.id, title: ticket.title });
    const url =
      ticket.column === "review" && ticket.pull_url
        ? ticket.pull_url
        : ticket.url;
    const result = await commands.openLink(url);
    if (result.status === "error") return setNotice(result.error);
    if (result.data.kind === "copy") {
      try {
        await navigator.clipboard.writeText(result.data.url);
      } catch {
        // The line below still names the page, and the maintainer can copy it from GitHub.
      }
      return setNotice(result.data.message);
    }
    setNotice(null);
  }

  if (unavailable !== null) {
    return (
      <p role="status" className="p-3 text-sm text-muted-foreground">
        {unavailable}
      </p>
    );
  }
  if (view === null) {
    return (
      <p className="p-3 text-sm text-muted-foreground">
        Reading the board from GitHub.
      </p>
    );
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col p-2">
      {view.held_until !== null && <Limit until={view.held_until} />}
      <KanbanBoard
        columns={COLUMNS}
        tickets={view.tickets}
        onMove={move}
        onSelect={(ticket) => void open(ticket)}
        notice={notice}
        className="flex-1"
        renderCard={(ticket) => (
          <>
            <span className="flex items-center gap-2 text-xs text-muted-foreground">
              <span className="font-mono">#{ticket.id}</span>
              {ticket.priority !== null && (
                <Badge variant="outline">P{ticket.priority}</Badge>
              )}
            </span>
            <span data-slot="ticket-title" className="text-lg font-semibold">
              {ticket.title}
            </span>
            {ticket.needs_you.length > 0 && (
              // The first line is what you read. The rest wait in its title.
              <StatusBadge
                status="needs"
                title={ticket.needs_you.join("\n")}
                className="max-w-full shrink"
              >
                {ticket.needs_you[0]}
              </StatusBadge>
            )}
            {ticket.column === "ready" && (
              // A press here is the button's own: the card's click would open the ticket.
              // biome-ignore lint/a11y/noStaticElementInteractions: only stops the click reaching the card
              <span
                className="mt-1 flex"
                onClick={(e) => e.stopPropagation()}
                onKeyDown={(e) => e.stopPropagation()}
              >
                <Button
                  type="button"
                  size="sm"
                  aria-label={`Start #${ticket.id}`}
                  onClick={() => void start(ticket)}
                >
                  Start
                </Button>
              </span>
            )}
          </>
        )}
      />
    </div>
  );
}

/**
 * The Board panel: `Panel` titled Board, its header counting every wait the
 * board holds, and the board as its body. The count is the board's own
 * `needs_you`, so it is the same number the tickets' markers add up to
 * (`docs/design/ux.md`, Attention).
 */
export function BoardPanel({
  onOpenWorker,
  ...controls
}: PanelControls & {
  onOpenWorker?: (ticket: { id: string; title: string }) => void;
}) {
  const waits = useBoard((s) => s.view?.needs_you ?? 0);

  return (
    <Panel title="Board" count={waits} {...controls}>
      <Board onOpenWorker={onOpenWorker} />
    </Panel>
  );
}
