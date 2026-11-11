import type { Channel } from "@tauri-apps/api/core";
import {
  commands,
  type InstallOutcome,
  type LogEntry,
  type SandboxAnswer,
  type SandboxView,
  type SessionEvent,
} from "@/bindings";

/**
 * The calls one chat makes, over the session it shows. A store holds one, so
 * two chats never share a session.
 */
export type Transport = {
  /** The session's log, oldest entry first. */
  history: () => Promise<
    { status: "ok"; data: LogEntry[] } | { status: "error"; error: string }
  >;
  /**
   * Send `text` to the session, streaming what the turn it starts says into
   * `channel`.
   *
   * A session that streams on the channel its `watch` was given is sent that one,
   * and a send that starts no turn of its own answers nothing. Answers the divider
   * the send logged, if any.
   */
  send: (
    text: string,
    channel: Channel<SessionEvent>,
  ) => Promise<string | null | undefined>;
  /** Send the interrupted last message again, with the same answer as `send`. */
  retry: (channel: Channel<SessionEvent>) => Promise<string | null | undefined>;
  /** Stop the turn in flight. */
  cancel: () => Promise<void>;
  /**
   * Interrupt the turn in flight and send `text` as the next one.
   *
   * Only a session the maintainer steers has it: a transport without one leaves the
   * chat's Stop and tell out.
   */
  stopAndTell?: (text: string) => Promise<void>;
  /** Answer the sandbox request `id`. */
  answer: (
    id: string,
    answer: SandboxAnswer,
  ) => Promise<
    { status: "ok"; data: null } | { status: "error"; error: string }
  >;
  /**
   * Follow a session that runs on its own: `load` hands it a channel before it
   * asks for the history, and every event the session sends after that goes to
   * the channel. The PM's transport has none, since a PM turn streams into the
   * channel of the `send` that started it.
   */
  watch?: (channel: Channel<SessionEvent>) => void;
  /** Stop sending to a channel `watch` was given, when its chat closes. */
  unwatch?: (channel: Channel<SessionEvent>) => Promise<void>;
  /**
   * What the session's sandbox last found. Only the PM's session has a sandbox
   * view so far: a transport without one leaves the chat's `sandbox` empty.
   */
  sandbox?: () => Promise<SandboxView>;
  /** Load kelpie's AppArmor profile. Only the PM's session has one to load. */
  install?: () => Promise<InstallOutcome>;
};

/** The PM's session, over the `pm_*` commands. */
export const pmTransport: Transport = {
  history: () => commands.pmHistory(),
  send: (text, channel) => commands.pmSend(text, channel),
  retry: (channel) => commands.pmRetry(channel),
  cancel: () => commands.pmCancel(),
  answer: (id, answer) => commands.pmAnswer(id, answer),
  sandbox: () => commands.pmSandbox(),
  install: () => commands.sandboxInstall(),
};
