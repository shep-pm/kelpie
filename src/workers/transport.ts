import { Channel } from "@tauri-apps/api/core";
import { commands, type SessionEvent } from "@/bindings";
import type { Transport } from "@/chat/transport";

const WAITS = "Sending to a worker waits for #153.";

/**
 * The session of the worker on `ticket`, over `workerWatch`.
 *
 * `workerWatch` answers the log and follows the worker live on the channel it
 * is given, so `watch` makes that one call and `history` answers from it. A
 * worker's own `send`, `retry`, `cancel` and `answer` wait for #153: each says
 * so, and none reaches a `pm_*` command.
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
    send: async () => {
      throw new Error(WAITS);
    },
    retry: async () => {
      throw new Error(WAITS);
    },
    cancel: async () => {
      throw new Error(WAITS);
    },
    answer: async () => ({ status: "error", error: WAITS }),
  };
}
