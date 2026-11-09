import { Channel } from "@tauri-apps/api/core";
import { create } from "zustand";
import {
  commands,
  type SessionEvent,
  type ToolStatus,
  type Usage,
} from "@/bindings";

export type Step = { id: string; title: string; status: ToolStatus };

export type TurnState = "running" | "done" | "failed" | "cancelled";

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
  /** Send `text` to the PM. Does nothing while a turn runs. */
  send: (text: string) => Promise<void>;
  /** Stop the turn in flight. */
  cancel: () => Promise<void>;
  /** Forget every turn. For tests. */
  reset: () => void;
};

function patch(turns: Turn[], id: number, change: (turn: Turn) => Turn) {
  return turns.map((turn) => (turn.id === id ? change(turn) : turn));
}

function apply(turn: Turn, event: SessionEvent): Turn {
  switch (event.kind) {
    case "text":
      return { ...turn, reply: turn.reply + event.delta };
    case "tool_step": {
      const step = { id: event.id, title: event.title, status: event.status };
      const at = turn.steps.findIndex((s) => s.id === event.id);
      const steps =
        at < 0
          ? [...turn.steps, step]
          : turn.steps.map((s, i) => (i === at ? step : s));
      return { ...turn, steps };
    }
    case "turn_ended":
      return {
        ...turn,
        usage: event.usage,
        state: event.stop_reason === "cancelled" ? "cancelled" : "done",
      };
    case "failed":
      // A `Failed` that a turn survives is followed by `TurnEnded`, which
      // moves the state on. The message stays under the turn either way.
      return { ...turn, state: "failed", error: event.message };
    default:
      return turn;
  }
}

let nextId = 1;

export const useChat = create<Chat>()((set, get) => ({
  turns: [],

  async send(text) {
    if (get().turns.some((turn) => turn.state === "running")) return;
    const id = nextId++;
    const turn: Turn = {
      id,
      user: text,
      reply: "",
      steps: [],
      usage: null,
      state: "running",
    };
    set((s) => ({ turns: [...s.turns, turn] }));

    const channel = new Channel<SessionEvent>();
    channel.onmessage = (event) =>
      set((s) => ({ turns: patch(s.turns, id, (t) => apply(t, event)) }));
    try {
      await commands.pmSend(text, channel);
    } catch (error) {
      set((s) => ({
        turns: patch(s.turns, id, (t) => ({
          ...t,
          state: "failed",
          error: String(error),
        })),
      }));
    }
  },

  async cancel() {
    await commands.pmCancel();
  },

  reset() {
    set({ turns: [] });
  },
}));
