/** What the settings page knows about a harness, by the id a settings file holds. */
export type HarnessInfo = {
  /** The name a person reads. */
  name: string;
  /** How the harness's login is kept and made, for a harness that has one. */
  login?: { key: string; command: string };
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
  },
  goose: { name: "goose" },
};

/** The name `harness_options` and the keychain know a harness by. */
export function askedAs(harness: string): string {
  return HARNESSES[harness]?.login?.key ?? harness;
}
