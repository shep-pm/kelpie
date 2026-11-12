import { useEffect, useState } from "react";
import {
  type Column,
  commands,
  type GateLine as GateLineView,
  type TicketView,
} from "@/bindings";
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

/**
 * Whether a Review card says a reviewer is reading its head, which is what opens the
 * read rather than the worker or the pull request. The engine words the line
 * `Reviewing, read 1 of 2` (`crates/kelpie-engine/src/gate/mod.rs`).
 */
const reviewing = (ticket: TicketView) =>
  ticket.gate?.text.startsWith("Reviewing") ?? false;

/**
 * The lines the status badge shows: the ticket's waits, less the merge yes when
 * the card's gate line already says it and carries the Merge button.
 */
const waitsOn = (ticket: TicketView) =>
  ticket.needs_you.filter(
    (line) => !(ticket.gate?.ready && line === MERGE_YES),
  );

/**
 * The one line a Review card shows for where the gate is, a link to the failing
 * run when it has one. The link goes through `onOpen`, so the OS's own broker
 * opens it and the webview never navigates (ADR 0001).
 */
function GateLine({
  gate,
  onOpen,
}: {
  gate: GateLineView;
  onOpen: (url: string) => void;
}) {
  const { url } = gate;
  return (
    <span data-slot="gate-line" className="text-sm text-muted-foreground">
      {url === null ? (
        gate.text
      ) : (
        <a
          href={url}
          className="underline underline-offset-2 hover:text-foreground"
          onClick={(e) => {
            // The card's click would open the pull request too
            e.preventDefault();
            e.stopPropagation();
            onOpen(url);
          }}
          onKeyDown={(e) => e.stopPropagation()}
        >
          {gate.text}
        </a>
      )}
    </span>
  );
}

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
 * on the maintainer. A Review ticket whose gate line says a reviewer is reading
 * its head calls `onOpenReviewer` instead. Clicking any other opens its pull
 * request in Review, else its issue.
 */
export function Board({
  onOpenWorker,
  onOpenReviewer,
}: {
  /** Open the worker on a ticket, which the PM panel's tabs answer. */
  onOpenWorker?: (ticket: { id: string; title: string }) => void;
  /** Open the reviewer reading a ticket, as `docs/design/review.md` shows it. */
  onOpenReviewer?: (ticket: { id: string; title: string }) => void;
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

  /** The yes to merging `ticket`: the same call the drop on Done makes. */
  async function merge(ticket: TicketView) {
    setNotice(null);
    const result = await commands.boardMove(ticket.id, "done");
    if (result.status === "error") return setNotice(result.error);
    // The yes is kept and the gate merges on its next pass, so the card stays in Review
    if (result.data.kind === "snap_back") setNotice(result.data.line);
  }

  async function openPage(url: string) {
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

  async function open(ticket: TicketView) {
    if (ticket.column === "review" && reviewing(ticket))
      // The gate says a reader is on this head now, and the read's own tab is
      // the only place to watch it (`docs/design/review.md`).
      return onOpenReviewer?.({ id: ticket.id, title: ticket.title });
    if (ticket.column === "building")
      return onOpenWorker?.({ id: ticket.id, title: ticket.title });
    if (ticket.column === "review" && workerWaits(ticket))
      // The worker's own needs-you line asks the maintainer to answer that
      // worker, so the card opens the tab where they can, even after the tab
      // was closed.
      return onOpenWorker?.({ id: ticket.id, title: ticket.title });
    return openPage(
      ticket.column === "review" && ticket.pull_url
        ? ticket.pull_url
        : ticket.url,
    );
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
            {ticket.gate && (
              <GateLine
                gate={ticket.gate}
                onOpen={(url) => void openPage(url)}
              />
            )}
            {waitsOn(ticket).length > 0 && (
              // The first line is what you read. The rest wait in its title.
              <StatusBadge
                status="needs"
                title={waitsOn(ticket).join("\n")}
                className="max-w-full shrink"
              >
                {waitsOn(ticket)[0]}
              </StatusBadge>
            )}
            {ticket.gate?.ready && (
              // A press here is the button's own: the card's click would open the pull request.
              // biome-ignore lint/a11y/noStaticElementInteractions: only stops the click reaching the card
              <span
                className="mt-1 flex"
                onClick={(e) => e.stopPropagation()}
                onKeyDown={(e) => e.stopPropagation()}
              >
                <Button
                  type="button"
                  size="sm"
                  aria-label={`Merge #${ticket.id}`}
                  onClick={() => void merge(ticket)}
                >
                  Merge
                </Button>
              </span>
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
  onOpenReviewer,
  ...controls
}: PanelControls & {
  onOpenWorker?: (ticket: { id: string; title: string }) => void;
  onOpenReviewer?: (ticket: { id: string; title: string }) => void;
}) {
  const waits = useBoard((s) => s.view?.needs_you ?? 0);

  return (
    <Panel title="Board" count={waits} {...controls}>
      <Board onOpenWorker={onOpenWorker} onOpenReviewer={onOpenReviewer} />
    </Panel>
  );
}
