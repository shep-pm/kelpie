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
  /** Send `text` and stream the turn into `channel`. Answers the divider the send logged, if any. */
  send: (
    text: string,
    channel: Channel<SessionEvent>,
  ) => Promise<string | null | undefined>;
  /** Send the interrupted last message again, with the same answer as `send`. */
  retry: (channel: Channel<SessionEvent>) => Promise<string | null | undefined>;
  /** Stop the turn in flight. */
  cancel: () => Promise<void>;
  /** Answer the sandbox request `id`. */
  answer: (
    id: string,
    answer: SandboxAnswer,
  ) => Promise<
    { status: "ok"; data: null } | { status: "error"; error: string }
  >;
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
