/** What the settings page knows about a harness, by the id a settings file holds. */
export type HarnessInfo = {
  /** The name a person reads. */
  name: string;
  /** How the harness's login is kept and made, for a harness that has one. */
  login?: { key: string; command: string };
  /** The bridge kelpie installs for it, for a harness that does not speak ACP itself. */
  bridge?: BridgeInfo;
};

/** One bridge kelpie installs and updates (`docs/design/bridge-updates.md`). */
export type BridgeInfo = {
  /** The id its versions, installs and setting are keyed by. */
  id: string;
  /** The name a person reads, which is the command kelpie runs. */
  name: string;
};

/**
 * Claude Code is the only harness with a login and a list of models in kelpie
 * yet (#12). A harness missing here is still offered: its fields stay text
 * fields, and the page says why it could not ask.
 */
export const HARNESSES: Record<string, HarnessInfo> = {
  "claude-code": {
    name: "Claude Code",
    login: { key: "claude", command: "claude setup-token" },
    bridge: { id: "claude-acp", name: "claude-agent-acp" },
  },
  goose: { name: "goose" },
};

/** The name `harness_options` and the keychain know a harness by. */
export function askedAs(harness: string): string {
  return HARNESSES[harness]?.login?.key ?? harness;
}
