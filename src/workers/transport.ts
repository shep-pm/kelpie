import { Channel } from "@tauri-apps/api/core";
import { commands, type NoteOutcome, type SessionEvent } from "@/bindings";
import type { Outcome, Transport } from "@/chat/transport";

/** Why a call a worker does not have says so. */
function refused(error: string): never {
  throw new Error(error);
}

/**
 * What became of a tell, which is the worker's own answer for it.
 *
 * `sent` is the text going out as a turn, and `queued` is the ticket's record keeping
 * it for the worker's next prompt. A call that never reached the worker says why.
 */
async function told(
  call: Promise<
    { status: "ok"; data: NoteOutcome } | { status: "error"; error: string }
  >,
): Promise<Outcome> {
  const answer = await call;
  if (answer.status === "error") refused(answer.error);
  return { told: answer.data };
}

/**
 * The session of the worker on `ticket`, over `workerWatch`.
 *
 * `workerWatch` answers the log and follows the worker live on the channel it is
 * given, so `watch` makes that one call and `history` reads from it. Every event of
 * the worker's turns arrives there, including the ones a Note starts, so `send` is
 * `worker_note` and streams nothing of its own.
 *
 * What `send` and `stopAndTell` answer is what the worker made of the text, which the
 * chat needs to show it: a Note the record kept started no turn, and a turn of it would
 * run with nothing ever streaming into it.
 */
export function workerTransport(ticket: string): Transport {
  let read: ReturnType<typeof commands.workerWatch> | null = null;
  return {
    watch: (channel) => {
      read ??= commands.workerWatch(ticket, channel);
    },
    unwatch: async (channel) => {
      // The feed hears an unwatch that beats its watch before it has the watcher to drop
      await read?.catch(() => {});
      await commands.workerUnwatch(ticket, channel);
    },
    history: async () => {
      // `load` watches first. A history asked for alone has no channel to follow on.
      read ??= commands.workerWatch(ticket, new Channel<SessionEvent>());
      return read;
    },
    send: (text) => told(commands.workerNote(ticket, text)),
    retry: () =>
      refused(
        "A worker whose turn was cut off resumes with Resume, not with its message again.",
      ),
    cancel: async () => {
      const stopped = await commands.workerCancel(ticket);
      if (stopped.status === "error") refused(stopped.error);
    },
    stopAndTell: (text) => told(commands.workerStopAndTell(ticket, text)),
    answer: (id, answer) => commands.workerAnswer(ticket, id, answer),
  };
}
