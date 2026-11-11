import { Channel } from "@tauri-apps/api/core";
import { commands, type SessionEvent } from "@/bindings";
import type { Transport } from "@/chat/transport";

/** Why a call a worker does not have says so. */
function refused(error: string): never {
  throw new Error(error);
}

/**
 * The session of the worker on `ticket`, over `workerWatch`.
 *
 * `workerWatch` answers the log and follows the worker live on the channel it is
 * given, so `watch` makes that one call and `history` reads from it. Every event of
 * the worker's turns arrives there, including the ones a Note starts, so `send` is
 * `worker_note` and streams nothing of its own.
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
    send: async (text) => {
      const told = await commands.workerNote(ticket, text);
      if (told.status === "error") refused(told.error);
      // A note starts no turn of its own to stream, and logs no divider
      return undefined;
    },
    retry: () =>
      refused(
        "A worker whose turn was cut off resumes with Resume, not with its message again.",
      ),
    cancel: async () => {
      const stopped = await commands.workerCancel(ticket);
      if (stopped.status === "error") refused(stopped.error);
    },
    stopAndTell: async (text) => {
      const told = await commands.workerStopAndTell(ticket, text);
      if (told.status === "error") refused(told.error);
    },
    answer: (id, answer) => commands.workerAnswer(ticket, id, answer),
  };
}
