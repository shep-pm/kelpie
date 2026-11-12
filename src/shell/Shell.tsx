import { KanbanSquare, MessageSquare } from "lucide-react";
import { useEffect, useState } from "react";
import { commands } from "@/bindings";
import { BoardPanel } from "@/board/Board";
import { useBoard } from "@/board/store";
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from "@/components/ui/resizable";
import { ReviewerTab } from "@/review/ReviewerTab";
import { WorkerTab, type WorkerTicket } from "@/workers/WorkerTab";
import { PanelStrip, PanelStripButton } from "./Panel";
import { PmPanel } from "./PmPanel";
import { registerTabBody, useTabs } from "./tabs";

type PanelId = "pm" | "board";

/** The narrowest a docked panel resizes to (`panel-min` in the design system). */
const PANEL_MIN = "320px";

/** Open the worker on `ticket` as a tab of the PM panel, or show it if it is open. */
function openWorker(ticket: WorkerTicket) {
  const id = `worker:${ticket.id}`;
  registerTabBody(id, () => <WorkerTab ticket={ticket} />);
  useTabs.getState().open({ id, title: `#${ticket.id}`, ticket: ticket.id });
}

/**
 * The reviewer reading `ticket`, which titles its tab.
 *
 * `null` when kelpie has seen no read of it, which is how a card that has moved on from
 * `Reviewing` still opens a tab.
 */
async function reviewerOf(ticket: string): Promise<string | null> {
  try {
    const answer = await commands.reviewReviewer(ticket);
    return answer.status === "ok" ? answer.data : null;
  } catch {
    return null;
  }
}

/**
 * Open the read of `ticket` as a tab of the PM panel beside its worker's, or show it if it
 * is open.
 *
 * The tab is titled with the reviewer, since the worker's tab of the same ticket sits
 * beside it and two tabs named `#14` are one too many.
 */
function openReviewer(ticket: WorkerTicket) {
  const id = `reviewer:${ticket.id}`;
  void reviewerOf(ticket.id).then((reviewer) => {
    registerTabBody(id, () => <ReviewerTab ticket={{ ...ticket, reviewer }} />);
    useTabs.getState().open({ id, title: reviewer ?? `#${ticket.id}` });
  });
}

/**
 * The window: the PM's panel on the left and the Board's on the right, each
 * folding to an icon strip button or taking the whole window.
 *
 * A folded panel is not mounted, so its body starts afresh when it reopens. The
 * board keeps streaming all the while, since the counts it feeds sit on the
 * folded panel's button and on the worker tabs beside the PM.
 */
export function Shell({ onSettings }: { onSettings: () => void }) {
  const [folded, setFolded] = useState<ReadonlySet<PanelId>>(new Set());
  const [maximised, setMaximised] = useState<PanelId | null>(null);

  // The window reads the board for as long as it is up, not just while the
  // Board panel is mounted, so a folded panel's count and the counts on the
  // worker tabs keep up with what the board last read.
  useEffect(() => {
    useBoard.getState().watch();
    return () => useBoard.getState().close();
  }, []);

  const boardWaits = useBoard((s) => s.view?.needs_you ?? 0);

  const shown = (id: PanelId) =>
    maximised === null ? !folded.has(id) : maximised === id;

  function fold(id: PanelId) {
    setFolded((now) => new Set(now).add(id));
  }
  function reopen(id: PanelId) {
    setFolded((now) => {
      const next = new Set(now);
      next.delete(id);
      return next;
    });
  }
  const controls = (id: PanelId) => ({
    maximised: maximised === id,
    onCollapse: () => fold(id),
    onMaximise: () => setMaximised((now) => (now === id ? null : id)),
  });

  const both = shown("pm") && shown("board");

  return (
    <div className="flex min-h-0 flex-1">
      <ResizablePanelGroup orientation="horizontal" className="min-w-0 flex-1">
        {shown("pm") && (
          <ResizablePanel id="pm" minSize={PANEL_MIN} defaultSize="50%">
            <PmPanel {...controls("pm")} onSettings={onSettings} />
          </ResizablePanel>
        )}
        {both && <ResizableHandle />}
        {shown("board") && (
          <ResizablePanel id="board" minSize={PANEL_MIN} defaultSize="50%">
            <BoardPanel
              {...controls("board")}
              onOpenWorker={openWorker}
              onOpenReviewer={openReviewer}
            />
          </ResizablePanel>
        )}
      </ResizablePanelGroup>
      {maximised === null && folded.size > 0 && (
        <PanelStrip>
          {folded.has("pm") && (
            <PanelStripButton
              title="PM"
              icon={<MessageSquare />}
              onOpen={() => reopen("pm")}
            />
          )}
          {folded.has("board") && (
            <PanelStripButton
              title="Board"
              icon={<KanbanSquare />}
              count={boardWaits}
              onOpen={() => reopen("board")}
            />
          )}
        </PanelStrip>
      )}
    </div>
  );
}
