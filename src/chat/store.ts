import { Channel } from "@tauri-apps/api/core";
import { create } from "zustand";
import {
  commands,
  type LogEntry,
  type SessionEvent,
  type ToolStatus,
  type Usage,
} from "@/bindings";

export type Step = { id: string; title: string; status: ToolStatus };

/** `interrupted`: the log has the message and no end, because kelpie quit mid-turn. */
export type TurnState =
  | "running"
  | "done"
  | "failed"
  | "cancelled"
  | "interrupted";

export type Turn = {
  id: number;
  user: string;
  reply: string;
  steps: Step[];
  usage: Usage | null;
  state: TurnState;
  error?: string;
};

type Chat = {
  turns: Turn[];
  /** Why the history could not be read, `null` when it could. */
  historyError: string | null;
  /**
   * Read the PM's history from kelpie's log, once. Every call returns the same
   * load, and `send` waits for it, so history always comes before a new turn.
   */
  load: () => Promise<void>;
  /** Send `text` to the PM. Does nothing while a turn runs. */
  send: (text: string) => Promise<void>;
  /** Send the interrupted last message again, in place of the interrupted turn. */
  retry: () => Promise<void>;
  /** Stop the turn in flight. */
  cancel: () => Promise<void>;
  /** Forget every turn. For tests. */
  reset: () => void;
};

function patch(turns: Turn[], id: number, change: (turn: Turn) => Turn) {
  return turns.map((turn) => (turn.id === id ? change(turn) : turn));
}

/** A turn a `Failed` ended goes on running when more of it arrives. */
function resumed(turn: Turn): TurnState {
  return turn.state === "failed" ? "running" : turn.state;
}

function apply(turn: Turn, event: SessionEvent): Turn {
  switch (event.kind) {
    case "text":
      return { ...turn, state: resumed(turn), reply: turn.reply + event.delta };
    case "started":
      return { ...turn, state: resumed(turn) };
    case "tool_step": {
      const step = { id: event.id, title: event.title, status: event.status };
      const at = turn.steps.findIndex((s) => s.id === event.id);
      const steps =
        at < 0
          ? [...turn.steps, step]
          : turn.steps.map((s, i) => (i === at ? step : s));
      return { ...turn, state: resumed(turn), steps };
    }
    case "turn_ended":
      return {
        ...turn,
        usage: event.usage,
        state: event.stop_reason === "cancelled" ? "cancelled" : "done",
      };
    case "failed":
      // The protocol cannot tell a `Failed` the turn survives from one that
      // ends it: both are the same event, and a fatal one is followed by
      // nothing. So it ends the turn, and any event that follows (text, a step,
      // `Started`) puts the turn back to running, so the maintainer is never
      // left with a Stop that does nothing. The message stays under the turn.
      return { ...turn, state: "failed", error: event.message };
    default:
      return turn;
  }
}

let nextId = 1;

function begin(user: string): Turn {
  return {
    id: nextId++,
    user,
    reply: "",
    steps: [],
    usage: null,
    state: "running",
  };
}

/**
 * Fold a log into turns: a user entry starts one, the events after it fill it
 * in. A turn the log never ends was cut off, so it is `interrupted`, and a turn
 * the maintainer retried shows once, as the retry.
 */
export function fold(entries: LogEntry[]): Turn[] {
  const turns: Turn[] = [];
  for (const entry of entries) {
    const last = turns.at(-1);
    if (entry.entry === "user") {
      if (last?.state === "running" && last.user === entry.text) turns.pop();
      turns.push(begin(entry.text));
    } else if (last) {
      turns[turns.length - 1] = apply(last, entry);
    }
  }
  return turns.map((turn) =>
    turn.state === "running" ? { ...turn, state: "interrupted" } : turn,
  );
}

let loading: Promise<void> | null = null;

export const useChat = create<Chat>()((set, get) => {
  /** Show `turn` as the last turn, and stream what `call` sends into it. */
  async function run(
    turn: Turn,
    call: (channel: Channel<SessionEvent>) => Promise<void>,
  ) {
    const { id } = turn;
    set((s) => ({ turns: [...s.turns, turn] }));

    const channel = new Channel<SessionEvent>();
    channel.onmessage = (event) =>
      set((s) => ({ turns: patch(s.turns, id, (t) => apply(t, event)) }));
    try {
      await call(channel);
    } catch (error) {
      set((s) => ({
        turns: patch(s.turns, id, (t) => ({
          ...t,
          state: "failed",
          error: String(error),
        })),
      }));
    }
  }

  return {
    turns: [],
    historyError: null,

    load() {
      loading ??= (async () => {
        try {
          const history = await commands.pmHistory();
          if (history.status === "error") {
            set({ historyError: history.error });
            return;
          }
          const past = fold(history.data);
          set((s) => ({ turns: [...past, ...s.turns], historyError: null }));
        } catch (error) {
          set({ historyError: String(error) });
        }
      })();
      return loading;
    },

    async send(text) {
      await get().load();
      if (get().turns.some((turn) => turn.state === "running")) return;
      await run(begin(text), (channel) => commands.pmSend(text, channel));
    },

    async retry() {
      await get().load();
      const last = get().turns.at(-1);
      if (last?.state !== "interrupted") return;
      set((s) => ({ turns: s.turns.slice(0, -1) }));
      await run(begin(last.user), (channel) => commands.pmRetry(channel));
    },

    async cancel() {
      await commands.pmCancel();
    },

    reset() {
      loading = null;
      set({ turns: [], historyError: null });
    },
  };
});
