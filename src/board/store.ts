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
   * Ask the engine to stream the board here, in place of any stream before. The
   * first event is the board, or why there is none.
   */
  watch: () => void;
  /** End the stream: the window drops what the engine still sends. */
  close: () => void;
  /** Forget everything. For tests. */
  reset: () => void;
};

/** The channel the engine streams to, `null` once closed. */
let open: Channel<BoardEvent> | null = null;

const empty = { view: null, unavailable: null } satisfies Partial<BoardState>;

/**
 * The last `BoardView` and the last `Unavailable`, filled by a channel from
 * `commands.boardWatch`.
 *
 * A channel has no close of its own, and the engine's poller ends when a send
 * to the window fails or a later watch replaces it. So `close` swaps the
 * channel's handler for one that drops events, and a `watch` that follows
 * replaces the poller.
 */
export const useBoard = create<BoardState>()((set) => ({
  ...empty,
  watch: () => {
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
    commands
      .boardWatch(channel)
      .then((result) => (result.status === "error" ? result.error : null))
      .catch((error: unknown) => String(error))
      .then((problem) => {
        if (problem !== null && open === channel) set({ unavailable: problem });
      });
  },
  close: () => {
    if (open) open.onmessage = () => {};
    open = null;
  },
  reset: () => {
    if (open) open.onmessage = () => {};
    open = null;
    set(empty);
  },
}));
