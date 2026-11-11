import { Channel } from "@tauri-apps/api/core";
import { create } from "zustand";
import { type BoardEvent, type BoardView, commands } from "@/bindings";

type BoardState = {
  /** The last board the window was sent, `null` until the first. */
  view: BoardView | null;
  /**
   * Why GitHub cannot be read, when the last thing the window heard says so. A
   * board that follows clears it.
   */
  unavailable: string | null;
  /**
   * Ask the engine to stream the board here, in place of any stream before, and
   * read it. The first event is the board, or why there is none. Every `watch`
   * wants its own `close`.
   */
  watch: () => void;
  /**
   * Ask again on a new channel, for the readers already counted, since a watch
   * that failed leaves the engine holding no channel to the window (no project
   * was picked yet). Adds no reader, and does nothing while none is reading.
   */
  rewatch: () => void;
  /**
   * Stop reading. The stream ends with the last reader: the window drops what
   * the engine still sends only once nothing is reading it.
   */
  close: () => void;
  /** Forget everything. For tests. */
  reset: () => void;
};

/** The channel the engine streams to, `null` once closed. */
let open: Channel<BoardEvent> | null = null;

/**
 * How many parts of the window are reading the board. `watch` adds one, `close`
 * takes one back, and only the last one ends the stream.
 *
 * The Shell counts waits for panels and worker tabs that outlive the board
 * body, so a Board panel folding must not stop the stream the rest of the
 * window still reads.
 */
let readers = 0;

/** The `boardWatch` call before, so the engine hears the watches in the order they were asked. */
let before: Promise<unknown> = Promise.resolve();

const empty = { view: null, unavailable: null } satisfies Partial<BoardState>;

/**
 * The last `BoardView` and the last `Unavailable`, filled by a channel from
 * `commands.boardWatch`.
 *
 * A channel has no close of its own, and the engine's poller ends when a send
 * to the window fails or a later watch replaces it. So `close` swaps the
 * channel's handler for one that drops events, and a `watch` that follows
 * replaces the poller. Until then a closed board's poller keeps reading once a
 * minute: the engine has no command to stop it.
 *
 * Each `boardWatch` waits for the one before to answer, since the engine
 * finishes two calls in flight in either order, and the later poller must be the
 * one that stays. A watch closed before its turn is never sent.
 *
 * Readers are counted, so a `close` ends the stream only once the last one has
 * closed. The Shell watches for as long as the window is up, which is what
 * keeps the counts on a folded panel and on a worker's tab current while the
 * board body comes and goes.
 */
export const useBoard = create<BoardState>()((set) => {
  /** Open a channel in place of any before, and send `boardWatch` with it in turn. */
  const openChannel = () => {
    const channel = new Channel<BoardEvent>();
    open = channel;
    channel.onmessage = (event) => {
      if (open !== channel) return;
      if (event.kind === "unavailable") {
        set({ unavailable: event.message });
      } else {
        const { kind: _kind, ...view } = event;
        set({ view, unavailable: null });
      }
    };
    const send = async (): Promise<string | null> => {
      if (open !== channel) return null;
      try {
        const result = await commands.boardWatch(channel);
        return result.status === "error" ? result.error : null;
      } catch (error) {
        return String(error);
      }
    };
    before = before.then(send).then((problem) => {
      if (problem !== null && open === channel) set({ unavailable: problem });
    });
  };

  return {
    ...empty,
    watch: () => {
      readers += 1;
      openChannel();
    },
    rewatch: () => {
      if (readers > 0) openChannel();
    },
    close: () => {
      readers = readers > 0 ? readers - 1 : 0;
      // Something else is still reading, so the stream stays open for it.
      if (readers > 0) return;
      if (open) open.onmessage = () => {};
      open = null;
    },
    reset: () => {
      readers = 0;
      if (open) open.onmessage = () => {};
      open = null;
      set(empty);
    },
  };
});
