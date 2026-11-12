import { Channel } from "@tauri-apps/api/core";
import { commands, type SessionEvent } from "@/bindings";
import type { Transport } from "@/chat/transport";

/** Why a call a watched read has no answer for says so. */
function refused(error: string): never {
  throw new Error(error);
}

/**
 * The reads of `ticket`, over `reviewWatch`.
 *
 * `reviewWatch` answers every read's log and follows the read in flight on the channel it
 * is given, so `watch` makes that one call and `history` reads from it. A read is watched
 * and never told: kelpie writes its prompt, and what it found reaches the worker as the
 * worker's next message (`docs/design/review.md`, What the maintainer sees), so nothing
 * here sends one.
 */
export function reviewerTransport(ticket: string): Transport {
  let read: ReturnType<typeof commands.reviewWatch> | null = null;
  return {
    watch: (channel) => {
      read ??= commands.reviewWatch(ticket, channel);
    },
    unwatch: async (channel) => {
      // The feed hears an unwatch that beats its watch before it has the watcher to drop
      await read?.catch(() => {});
      await commands.reviewUnwatch(ticket, channel);
    },
    history: async () => {
      // `load` watches first. A history asked for alone has no channel to follow on.
      read ??= commands.reviewWatch(ticket, new Channel<SessionEvent>());
      return read;
    },
    send: () =>
      refused(
        "A reviewer's read is the pass's own turn, and it takes no message.",
      ),
    retry: () =>
      refused(
        "A read that was cut off runs again when the pass next steps the ticket.",
      ),
    cancel: () =>
      refused("A read is stopped by the pass, not by the tab watching it."),
    answer: () =>
      refused("A reviewer's sandbox asks the maintainer for nothing."),
  };
}
