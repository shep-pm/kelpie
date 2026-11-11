import { Channel } from "@tauri-apps/api/core";
import { create, type StoreApi, type UseBoundStore } from "zustand";
import type {
  InstallOutcome,
  LogEntry,
  NoteOutcome,
  SandboxAnswer,
  SandboxView,
  SessionEvent,
  ToolStatus,
  Usage,
} from "@/bindings";
import { type Outcome, pmTransport, type Transport } from "./transport";

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

/**
 * A message the maintainer sent to a session that is not a turn of its own.
 *
 * A worker's Note sent while its turn runs waits in the ticket's record, and shows
 * under the turn it waits behind. `line` is what the session answered it with, and
 * `error` is why the message never reached the session, in kelpie's own words.
 */
export type Note = {
  text: string;
  /**
   * What kelpie did with it, in one line.
   *
   * Absent when the message never reached the session, which `error` says instead.
   */
  line?: string;
  error?: string;
};

/** A Note the worker reads when the turn it is running ends. */
const QUEUED = "Queued, the worker reads it when its turn ends";

/** A Note the record kept for a worker that has no turn running. */
const KEPT = "Queued, kelpie sends it when the worker next runs";

/** What a Stop and tell sent as the prompt of the turn that follows. */
const NEXT_PROMPT = "Sent as the worker's next prompt";

/**
 * What the session answered a tell with, in the one line the chat shows.
 *
 * Only a tell behind a running turn uses it: a Note the record kept between turns says
 * so with `KEPT`, under [`kept`].
 */
function line(told: NoteOutcome | undefined): string {
  return told === "sent" ? NEXT_PROMPT : QUEUED;
}

/**
 * Put the text `run` opened a turn for back where the session kept it.
 *
 * The turn goes, since nothing streams into a turn kelpie started no work for, and the
 * note goes under the turn before it. A chat whose session ran no turn before this one
 * keeps the note on its own turn, which ends there: the session has the text, and no
 * turn of it has started.
 */
function kept(turns: Turn[], id: number, text: string): Turn[] {
  const note = { text, line: KEPT };
  const rest = turns.filter((turn) => turn.id !== id);
  const last = rest.at(-1);
  if (!last) {
    return turns.map((turn) =>
      turn.id === id
        ? { ...turn, state: "done" as const, notes: [...turn.notes, note] }
        : turn,
    );
  }
  return rest.map((turn) =>
    turn.id === last.id ? { ...turn, notes: [...turn.notes, note] } : turn,
  );
}

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
  /** Notes sent while this turn ran, oldest first. */
  notes: Note[];
  usage: Usage | null;
  state: TurnState;
  error?: string;
};

export type Chat = {
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
   * An install waits on its password prompt. A loaded profile ends the PM's
   * session, so no message is sent meanwhile: it would be cut off.
   */
  installing: boolean;
  /**
   * Read the PM's history from kelpie's log, once. Every call returns the same
   * load, and `send` waits for it, so history always comes before a new turn.
   */
  load: () => Promise<void>;
  /** Send `text` to the PM. Does nothing while a turn runs or an install waits. */
  send: (text: string) => Promise<void>;
  /**
   * Send `text` to a session the maintainer steers.
   *
   * While a turn runs it is a Note under that turn. Between turns it is the turn the
   * session runs next, unless the session keeps the text in its record, which the note
   * says either way.
   */
  note: (text: string) => Promise<void>;
  /**
   * Interrupt the turn in flight and send `text` as the next one.
   *
   * The text shows under the turn it interrupts, as sent or as kept, which is what the
   * session answered. A session that has no turn to interrupt is told as a Note, and a
   * transport that cannot be steered sends nothing.
   */
  stopAndTell: (text: string) => Promise<void>;
  /** Send the interrupted last message again, in place of the interrupted turn. */
  retry: () => Promise<void>;
  /**
   * Send the failed last turn's message again, as a new turn in its place, so
   * the maintainer need not retype it after fixing what stopped it. Does
   * nothing unless the last turn failed, so a second call while the first runs
   * sends nothing.
   */
  resend: () => Promise<void>;
  /**
   * Load kelpie's AppArmor profile, which ends the PM's session once it is
   * loaded. A finished install clears what the sandbox found and sends the
   * failed last message again, so the PM starts sandboxed without the message
   * typed twice. A second call while one runs shares its answer and sends
   * nothing more.
   */
  install: () => Promise<InstallOutcome>;
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
  /**
   * Stop following what the transport's `watch` sends. Reading the history
   * again after it needs a new chat.
   */
  close: () => void;
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

/** Events that carry a turn's own content, and so open one when none is running. */
const OPENS_TURN: ReadonlySet<SessionEvent["kind"]> = new Set([
  "started",
  "text",
  "tool_step",
  "sandbox_request",
]);

/**
 * Add what a watched session sent to the turn it belongs to. A turn the log
 * never ended was shown as `interrupted`, and the session going on shows it was
 * only running. The feed carries no user entry, so content that follows a turn
 * that ended opens a new turn, with no message of its own.
 */
function follow(turns: Turn[], event: SessionEvent): Turn[] {
  const last = turns.at(-1);
  if (!last) return turns;
  const ended = last.state === "done" || last.state === "cancelled";
  if (ended && OPENS_TURN.has(event.kind)) {
    return [...turns, apply(begin(""), event)];
  }
  const turn: Turn =
    last.state === "interrupted" ? { ...last, state: "running" } : last;
  return [...turns.slice(0, -1), apply(turn, event)];
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
    notes: [],
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

/** One chat's store, over the session `transport` reaches. */
export type ChatStore = UseBoundStore<StoreApi<Chat>>;

/**
 * Make a chat over `transport`. Each store holds its own turns, its own load
 * and its own install, so two chats never share a session.
 */
export function createChat(transport: Transport): ChatStore {
  let loading: Promise<void> | null = null;

  /** The channel the transport's `watch` was given, `null` before it and after `close`. */
  let watching: Channel<SessionEvent> | null = null;

  /** The install in flight, so a second click shares it instead of asking for a second password. */
  let installing: Promise<InstallOutcome> | null = null;

  /** Which look at the sandbox is the latest, so a slow answer never overwrites a newer one. */
  let lookedAt = 0;

  return create<Chat>()((set, get) => {
    /** Ask what the PM's sandbox found, and keep it, unless a newer look came first. */
    async function lookAtSandbox() {
      if (!transport.sandbox) return;
      const look = ++lookedAt;
      try {
        const sandbox = await transport.sandbox();
        if (look === lookedAt) set({ sandbox });
      } catch {
        // The turn's own error is still on screen, which is all the chat showed before this
      }
    }

    /**
     * Tell the turn `id` `text` with `call`, and show what the session answered.
     *
     * The line is the session's own answer, since only it knows whether the text went
     * out or is kept. A call that never reached the session says so instead: the message
     * is on the screen and the session has it not.
     */
    async function tell(
      id: number,
      text: string,
      call: () => Promise<Outcome>,
    ) {
      try {
        const answer = await call();
        set((s) => ({
          turns: patch(s.turns, id, (t) => ({
            ...t,
            notes: [...t.notes, { text, line: line(answer.told) }],
          })),
        }));
      } catch (error) {
        set((s) => ({
          turns: patch(s.turns, id, (t) => ({
            ...t,
            notes: [...t.notes, { text, error: String(error) }],
          })),
        }));
      }
    }

    /**
     * Show `turn` as the last turn, and stream what `call` sends into it.
     *
     * `note` is the text of a message a session the maintainer steers may keep instead
     * of running. When `call` answers `queued`, `turn` goes back and the text is kept as
     * a note under the turn before it, so the maintainer is never shown a turn that runs
     * with nothing ever streaming into it.
     */
    async function run(
      turn: Turn,
      call: (channel: Channel<SessionEvent>) => Promise<Outcome>,
      note?: string,
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
        const answer = await call(channel);
        if (note !== undefined && answer.told === "queued") {
          set((s) => ({ turns: kept(s.turns, id, note) }));
          return;
        }
        // A send that switched the harness answers with the divider it logged
        const divider = answer.divider;
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
      installing: false,

      load() {
        loading ??= (async () => {
          // What the session sends while the history is read waits for it, so it lands on the last turn
          let held: SessionEvent[] | null = [];
          if (transport.watch) {
            const channel = new Channel<SessionEvent>();
            channel.onmessage = (event) => {
              if (held) held.push(event);
              else set((s) => ({ turns: follow(s.turns, event) }));
            };
            watching = channel;
            transport.watch(channel);
          }
          try {
            const history = await transport.history();
            if (history.status === "error") {
              set({ historyError: history.error });
              return;
            }
            const past = fold(history.data);
            set((s) => ({ turns: [...past, ...s.turns], historyError: null }));
            for (const event of held.splice(0)) {
              set((s) => ({ turns: follow(s.turns, event) }));
            }
          } catch (error) {
            set({ historyError: String(error) });
          } finally {
            held = null;
          }
        })();
        return loading;
      },

      async send(text) {
        await get().load();
        if (get().installing) return;
        if (get().turns.some((turn) => turn.state === "running")) return;
        await run(begin(text), (channel) => transport.send(text, channel));
      },

      async note(text) {
        await get().load();
        if (get().installing) return;
        const running = get().turns.at(-1);
        if (running?.state === "running") {
          // A turn runs, so the note waits behind it, and the session says when it is read
          await tell(running.id, text, () =>
            // A steered session streams on the channel its `watch` was given, and a
            // note starts no turn of its own to stream into a fresh one.
            transport.send(text, watching ?? new Channel<SessionEvent>()),
          );
          return;
        }
        // Between turns the note is the session's next turn, unless it keeps the text
        await run(
          begin(text),
          (channel) => transport.send(text, channel),
          text,
        );
      },

      async stopAndTell(text) {
        await get().load();
        const running = get().turns.at(-1);
        if (running?.state !== "running") return get().note(text);
        const steered = transport.stopAndTell;
        if (!steered) return;
        await tell(running.id, text, () => steered(text));
      },

      async retry() {
        await get().load();
        const last = get().turns.at(-1);
        if (last?.state !== "interrupted") return;
        set((s) => ({ turns: s.turns.slice(0, -1) }));
        await run(begin(last.user, last.dividers), (channel) =>
          transport.retry(channel),
        );
      },

      async resend() {
        await get().load();
        const last = get().turns.at(-1);
        if (last?.state !== "failed") return;
        set((s) => ({ turns: s.turns.slice(0, -1) }));
        await run(
          begin(last.user, last.dividers),
          (channel) => transport.send(last.user, channel),
          last.user,
        );
      },

      install() {
        const load = transport.install;
        if (!load) {
          return Promise.resolve<InstallOutcome>({
            kind: "failed",
            message: "This session has no sandbox profile to install.",
          });
        }
        installing ??= (async (): Promise<InstallOutcome> => {
          set({ installing: true });
          try {
            const outcome = await load();
            if (outcome.kind === "installed") {
              // The session that found the refusal is gone, so its record is too
              await lookAtSandbox();
              await get().resend();
            }
            return outcome;
          } catch (error) {
            return { kind: "failed", message: String(error) };
          } finally {
            installing = null;
            set({ installing: false });
          }
        })();
        return installing;
      },

      async cancel() {
        await transport.cancel();
      },

      async answer(id, answer) {
        set((s) => ({
          turns: patchRequest(s.turns, id, (asked) => ({ ...asked, answer })),
        }));
        const done = await transport.answer(id, answer);
        if (done.status === "error") {
          set((s) => ({
            turns: patchRequest(s.turns, id, (asked) => ({
              ...asked,
              error: done.error,
            })),
          }));
        }
      },

      close() {
        const channel = watching;
        watching = null;
        if (!channel) return;
        channel.onmessage = () => {};
        void transport.unwatch?.(channel)?.catch(() => {});
      },

      reset() {
        loading = null;
        installing = null;
        lookedAt++;
        set({
          turns: [],
          historyError: null,
          sandbox: null,
          installing: false,
        });
      },
    };
  });
}

/** The PM's chat. */
export const useChat = createChat(pmTransport);
