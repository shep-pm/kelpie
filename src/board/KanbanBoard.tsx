/**
 * Tickets as columns, with a drag and a Move to menu that ask `onMove` where a
 * ticket goes.
 *
 * Lifted from chelone's `kanban-board.tsx` (`eb149dee4`, MIT); `docs/lifted.md`
 * has what came over and what did not.
 */
import { Accessibility, PointerSensor } from "@dnd-kit/dom";
import {
  DragDropProvider,
  type DragEndEvent,
  useDraggable,
  useDroppable,
} from "@dnd-kit/react";
import { ArrowRightLeftIcon } from "lucide-react";
import {
  type ComponentProps,
  type KeyboardEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useState,
} from "react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";

/** One column, named by `id`. */
export type KanbanColumn<C extends string = string> = {
  id: C;
  label: string;
};

/** What a ticket needs to sit on the board: an id, a title for its Move button, and its column. */
export type KanbanTicket<C extends string = string> = {
  id: string;
  title: string;
  column: C;
};

/**
 * What a move came to. `moved` keeps the ticket in the column it was dropped
 * on; `snap_back` puts it back and shows `line`.
 */
export type MoveOutcome =
  | { kind: "moved" }
  | { kind: "snap_back"; line: string };

type Props<C extends string, T extends KanbanTicket<C>> = {
  /** The columns, in order. A ticket whose column is not listed is not shown. */
  columns: readonly KanbanColumn<C>[];
  /** The tickets, in the order a column shows them. A new array is a new board. */
  tickets: readonly T[];
  /**
   * Ask where `ticket` may go. The ticket shows in `to` until this resolves, and
   * goes back if the answer is a snap back or the promise rejects.
   */
  onMove: (ticket: T, to: C) => Promise<MoveOutcome>;
  /** Called with the ticket when its card is clicked, or Enter or Space is pressed on it. */
  onSelect?: (ticket: T) => void;
  /** Replaces the title inside a card. */
  renderCard?: (ticket: T) => ReactNode;
  /** A line for the status area when no move has one. */
  notice?: string | null;
  className?: string;
};

// Enter and Space select a card, so the keyboard sensor stays off: the "Move to"
// menu is the keyboard path.
const SENSORS = [PointerSensor];
// Without the keyboard sensor, the Accessibility plugin would still tag a card
// "draggable" or "disabled" and tell a screen reader to press Space, which selects.
const PLUGINS: ComponentProps<typeof DragDropProvider>["plugins"] = (
  defaults,
) => defaults.filter((plugin) => plugin !== Accessibility);

/** The card area of a column: a drop target that marks itself while a card is over it. */
function KanbanColumnCards({
  columnId,
  children,
}: {
  columnId: string;
  children: ReactNode;
}) {
  const { ref, isDropTarget } = useDroppable({ id: columnId });
  return (
    <div
      ref={ref}
      data-slot="kanban-column-cards"
      data-over={isDropTarget ? "" : undefined}
      className="flex min-h-16 flex-col gap-2 rounded-md data-[over]:bg-accent"
    >
      {children}
    </div>
  );
}

/** A card that a pointer drags, whether mouse, touch or pen. */
function KanbanCardShell({
  ticketId,
  children,
  ...props
}: {
  ticketId: string;
  children: ReactNode;
} & Omit<ComponentProps<"div">, "ref" | "draggable">) {
  const { ref, isDragging } = useDraggable({ id: ticketId });
  return (
    <div
      ref={ref}
      data-slot="kanban-card"
      data-ticket-id={ticketId}
      data-dragging={isDragging ? "" : undefined}
      {...props}
    >
      {children}
    </div>
  );
}

/** A ticket dropped on a column and not yet back in the tickets. */
type Pending<C extends string> = { to: C; settled: boolean };

/**
 * The tickets as columns, one per `columns` entry, with a card per ticket. A
 * card moves to another column by dragging it, or with its "Move to" menu for
 * keyboard users. Either calls `onMove`, and the card changes column at once.
 * It stays there once the move is `moved`, until `tickets` changes, and goes
 * back with the answer's line when the move snaps back or fails. A drag uses
 * pointer events, so mouse, touch and pen all drag; a touch drags after a
 * short hold, so a swipe still scrolls the board.
 */
export function KanbanBoard<C extends string, T extends KanbanTicket<C>>({
  columns,
  tickets,
  onMove,
  onSelect,
  renderCard,
  notice,
  className,
}: Props<C, T>) {
  const [pending, setPending] = useState<ReadonlyMap<string, Pending<C>>>(
    new Map(),
  );
  const [line, setLine] = useState<string | null>(null);

  // A new board answers every move that has settled. One still waiting stays.
  // biome-ignore lint/correctness/useExhaustiveDependencies: `tickets` is the trigger, not a value read
  useEffect(() => {
    setPending((now) => {
      if (![...now.values()].some((p) => p.settled)) return now;
      return new Map([...now].filter(([, p]) => !p.settled));
    });
  }, [tickets]);

  // A notice that arrives is the newest word, so it replaces an older move's line.
  useEffect(() => {
    if (notice) setLine(null);
  }, [notice]);

  const buckets = useMemo(
    () =>
      columns.map((column) => ({
        column,
        tickets: tickets.filter(
          (t) => (pending.get(t.id)?.to ?? t.column) === column.id,
        ),
      })),
    [columns, tickets, pending],
  );

  const move = useCallback(
    async (ticket: T, to: C) => {
      setLine(null);
      setPending((now) => new Map(now).set(ticket.id, { to, settled: false }));
      let outcome: MoveOutcome;
      try {
        outcome = await onMove(ticket, to);
      } catch (error) {
        outcome = {
          kind: "snap_back",
          line: error instanceof Error ? error.message : String(error),
        };
      }
      setPending((now) => {
        const next = new Map(now);
        if (outcome.kind === "moved")
          next.set(ticket.id, { to, settled: true });
        else next.delete(ticket.id);
        return next;
      });
      if (outcome.kind === "snap_back") setLine(outcome.line);
    },
    [onMove],
  );

  const onDragEnd = ({ canceled, operation }: DragEndEvent) => {
    const { source, target } = operation;
    if (canceled || !source || !target) return;
    const ticket = tickets.find((t) => t.id === String(source.id));
    if (!ticket) return;
    const from = pending.get(ticket.id)?.to ?? ticket.column;
    const to = columns.find((c) => c.id === String(target.id));
    if (to && to.id !== from) void move(ticket, to.id);
  };

  return (
    <DragDropProvider sensors={SENSORS} plugins={PLUGINS} onDragEnd={onDragEnd}>
      <div className={cn("flex min-h-0 min-w-0 flex-col gap-2", className)}>
        <p
          role="status"
          className="min-h-5 shrink-0 px-1 text-sm text-muted-foreground"
        >
          {line ?? notice}
        </p>
        <div
          data-slot="kanban-board"
          className="flex min-h-0 flex-1 items-start gap-4 overflow-auto pb-2"
        >
          {buckets.map(({ column, tickets: inColumn }) => (
            // biome-ignore lint/a11y/useSemanticElements: a fieldset would style as a form group; the column is a named region of cards
            <div
              key={column.id}
              role="group"
              aria-label={column.label}
              data-slot="kanban-column"
              data-column-id={column.id}
              className="flex w-72 shrink-0 flex-col gap-2 rounded-lg bg-muted/50 p-2"
            >
              <div className="flex items-center justify-between px-1 text-sm font-medium">
                <span>{column.label}</span>
                <span
                  data-slot="kanban-column-count"
                  className="text-muted-foreground"
                >
                  {inColumn.length}
                </span>
              </div>
              <KanbanColumnCards columnId={column.id}>
                {inColumn.length === 0 ? (
                  <p
                    data-slot="kanban-empty"
                    className="px-1 py-2 text-sm text-muted-foreground"
                  >
                    No tickets
                  </p>
                ) : (
                  inColumn.map((ticket) => (
                    <KanbanCardShell
                      key={ticket.id}
                      ticketId={ticket.id}
                      role="button"
                      tabIndex={0}
                      className="flex cursor-pointer items-start gap-2 rounded-md border bg-card p-3 text-sm shadow-xs data-[dragging]:opacity-50"
                      onClick={() => onSelect?.(ticket)}
                      onKeyDown={(e: KeyboardEvent<HTMLElement>) => {
                        if (e.target !== e.currentTarget) return;
                        if (e.key !== "Enter" && e.key !== " ") return;
                        e.preventDefault();
                        onSelect?.(ticket);
                      }}
                    >
                      <div className="flex min-w-0 flex-1 flex-col gap-1">
                        {renderCard ? (
                          renderCard(ticket)
                        ) : (
                          <span
                            data-slot="kanban-card-title"
                            className="font-medium"
                          >
                            {ticket.title}
                          </span>
                        )}
                      </div>
                      {/* Menu items are portalled, but React events still bubble to the card. */}
                      {/* biome-ignore lint/a11y/noStaticElementInteractions: only stops the menu's events reaching the card */}
                      <div
                        onClick={(e) => e.stopPropagation()}
                        onKeyDown={(e) => e.stopPropagation()}
                      >
                        <DropdownMenu>
                          <DropdownMenuTrigger asChild>
                            <Button
                              type="button"
                              variant="ghost"
                              size="icon"
                              data-slot="kanban-card-move"
                              className="size-6"
                              aria-label={`Move ${ticket.title}`}
                            >
                              <ArrowRightLeftIcon aria-hidden="true" />
                            </Button>
                          </DropdownMenuTrigger>
                          <DropdownMenuContent align="end">
                            {columns
                              .filter((other) => other.id !== column.id)
                              .map((other) => (
                                <DropdownMenuItem
                                  key={other.id}
                                  onSelect={() => void move(ticket, other.id)}
                                >
                                  {other.label}
                                </DropdownMenuItem>
                              ))}
                          </DropdownMenuContent>
                        </DropdownMenu>
                      </div>
                    </KanbanCardShell>
                  ))
                )}
              </KanbanColumnCards>
            </div>
          ))}
        </div>
      </div>
    </DragDropProvider>
  );
}
