import { Channel } from "@tauri-apps/api/core";
import { create } from "zustand";
import {
  commands,
  type LogEntry,
  type SandboxAnswer,
  type SandboxView,
  type SessionEvent,
  type ToolStatus,
  type Usage,
} from "@/bindings";

export type Step = { id: string; title: string; status: ToolStatus };

/**
 * A host the PM's sandbox refused, which the maintainer answers.
 *
 * `answer` is `null` while it waits. A request the turn ended without an answer
 * is denied, which is what the engine did with it.
 */
export type SandboxRequest = {
  id: string;
  host: string;
  why: string;
  answer: SandboxAnswer | null;
  /** Why the answer did not fully land, in kelpie's own words. */
  error?: string;
};

/** `interrupted`: the log has the message and no end, because kelpie quit mid-turn. */
export type TurnState =
  | "running"
  | "done"
  | "failed"
  | "cancelled"
  | "interrupted";

export type Turn = {
  id: number;
  /**
   * What kelpie wrote above this message because it started a new session: a
   * switch of harness, or a resume the harness refused. Oldest first.
   */
  dividers: string[];
  user: string;
  reply: string;
  steps: Step[];
  sandboxRequests: SandboxRequest[];
  usage: Usage | null;
  state: TurnState;
  error?: string;
};

type Chat = {
  turns: Turn[];
  /** Why the history could not be read, `null` when it could. */
  historyError: string | null;
  /**
   * What the PM's sandbox last found, asked for each time a send ends, whether
   * it failed or not. `null` until one does. `unsandboxed` is what the badge
   * in the PM's header reads.
   */
  sandbox: SandboxView | null;
  /**
   * Read the PM's history from kelpie's log, once. Every call returns the same
   * load, and `send` waits for it, so history always comes before a new turn.
   */
  load: () => Promise<void>;
  /** Send `text` to the PM. Does nothing while a turn runs. */
  send: (text: string) => Promise<void>;
  /** Send the interrupted last message again, in place of the interrupted turn. */
  retry: () => Promise<void>;
  /**
   * Send the failed last turn's message again, as a new turn in its place, so
   * the maintainer need not retype it after fixing what stopped it. Does
   * nothing unless the last turn failed, so a second call while the first runs
   * sends nothing.
   */
  resend: () => Promise<void>;
  /** Stop the turn in flight. */
  cancel: () => Promise<void>;
  /**
   * Answer the sandbox request `id` with `answer`.
   *
   * The answer is shown at once: the engine takes it before this call returns,
   * and only saving a *Allow for this project* can still fail, which the request
   * then says in its own line.
   */
  answer: (id: string, answer: SandboxAnswer) => Promise<void>;
  /** Forget every turn. For tests. */
  reset: () => void;
};

function patch(turns: Turn[], id: number, change: (turn: Turn) => Turn) {
  return turns.map((turn) => (turn.id === id ? change(turn) : turn));
}

/** Change the sandbox request `id`, in whichever turn holds it. */
function patchRequest(
  turns: Turn[],
  id: string,
  change: (asked: SandboxRequest) => SandboxRequest,
) {
  return turns.map((turn) =>
    turn.sandboxRequests.some((asked) => asked.id === id)
      ? {
          ...turn,
          sandboxRequests: turn.sandboxRequests.map((asked) =>
            asked.id === id ? change(asked) : asked,
          ),
        }
      : turn,
  );
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
    case "sandbox_request": {
      const asked: SandboxRequest = {
        id: event.id,
        host: event.host,
        why: event.why,
        answer: null,
      };
      return {
        ...turn,
        state: resumed(turn),
        sandboxRequests: [...turn.sandboxRequests, asked],
      };
    }
    case "turn_ended":
      // The engine answers every request the turn left waiting as denied, so the
      // maintainer is never offered a button that no longer reaches anything.
      return {
        ...turn,
        usage: event.usage,
        state: event.stop_reason === "cancelled" ? "cancelled" : "done",
        sandboxRequests: turn.sandboxRequests.map((asked) =>
          asked.answer === null ? { ...asked, answer: "deny" } : asked,
        ),
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

function begin(user: string, dividers: string[] = []): Turn {
  return {
    id: nextId++,
    dividers,
    user,
    reply: "",
    steps: [],
    sandboxRequests: [],
    usage: null,
    state: "running",
  };
}

/**
 * Fold a log into turns: a user entry starts one, the events after it fill it
 * in. A turn the log never ends was cut off, so it is `interrupted`, and a turn
 * the maintainer retried shows once, as the retry.
 *
 * A divider belongs to the turn whose message started the new session. A
 * switch of harness is logged just before that message, so a divider followed
 * by a user entry waits for it. A refused resume is logged inside the turn.
 */
export function fold(entries: LogEntry[]): Turn[] {
  const turns: Turn[] = [];
  let waiting: string[] = [];
  entries.forEach((entry, at) => {
    const last = turns.at(-1);
    if (entry.entry === "divider") {
      if (last && entries[at + 1]?.entry !== "user") {
        turns[turns.length - 1] = {
          ...last,
          dividers: [...last.dividers, entry.text],
        };
      } else {
        waiting = [...waiting, entry.text];
      }
    } else if (entry.entry === "user") {
      let dividers = waiting;
      if (last?.state === "running" && last.user === entry.text) {
        dividers = [...(turns.pop()?.dividers ?? []), ...waiting];
      }
      waiting = [];
      turns.push(begin(entry.text, dividers));
    } else if (last) {
      turns[turns.length - 1] = apply(last, entry);
    }
  });
  return turns.map((turn) =>
    turn.state === "running" ? { ...turn, state: "interrupted" } : turn,
  );
}

let loading: Promise<void> | null = null;

/** Which look at the sandbox is the latest, so a slow answer never overwrites a newer one. */
let lookedAt = 0;

export const useChat = create<Chat>()((set, get) => {
  /** Ask what the PM's sandbox found, and keep it, unless a newer look came first. */
  async function lookAtSandbox() {
    const look = ++lookedAt;
    try {
      const sandbox = await commands.pmSandbox();
      if (look === lookedAt) set({ sandbox });
    } catch {
      // The turn's own error is still on screen, which is all the chat showed before this
    }
  }

  /** Show `turn` as the last turn, and stream what `call` sends into it. */
  async function run(
    turn: Turn,
    call: (
      channel: Channel<SessionEvent>,
    ) => Promise<string | null | undefined>,
  ) {
    const { id } = turn;
    set((s) => ({ turns: [...s.turns, turn] }));

    const channel = new Channel<SessionEvent>();
    channel.onmessage = (event) => {
      set((s) => ({ turns: patch(s.turns, id, (t) => apply(t, event)) }));
      if (event.kind === "failed") void lookAtSandbox();
      // A send that ends has opened its session, so what the start found is current
      if (event.kind === "turn_ended") void lookAtSandbox();
    };
    try {
      // A send that switched the harness answers with the divider it logged
      const divider = await call(channel);
      if (divider) {
        set((s) => ({
          turns: patch(s.turns, id, (t) => ({
            ...t,
            dividers: [...t.dividers, divider],
          })),
        }));
      }
    } catch (error) {
      set((s) => ({
        turns: patch(s.turns, id, (t) => ({
          ...t,
          state: "failed",
          error: String(error),
        })),
      }));
      void lookAtSandbox();
    }
  }

  return {
    turns: [],
    historyError: null,
    sandbox: null,

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
      await run(begin(last.user, last.dividers), (channel) =>
        commands.pmRetry(channel),
      );
    },

    async resend() {
      await get().load();
      const last = get().turns.at(-1);
      if (last?.state !== "failed") return;
      set((s) => ({ turns: s.turns.slice(0, -1) }));
      await run(begin(last.user, last.dividers), (channel) =>
        commands.pmSend(last.user, channel),
      );
    },

    async cancel() {
      await commands.pmCancel();
    },

    async answer(id, answer) {
      set((s) => ({
        turns: patchRequest(s.turns, id, (asked) => ({ ...asked, answer })),
      }));
      const done = await commands.pmAnswer(id, answer);
      if (done.status === "error") {
        set((s) => ({
          turns: patchRequest(s.turns, id, (asked) => ({
            ...asked,
            error: done.error,
          })),
        }));
      }
    },

    reset() {
      loading = null;
      lookedAt++;
      set({ turns: [], historyError: null, sandbox: null });
    },
  };
});
