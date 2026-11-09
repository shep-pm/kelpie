// Typed wrappers over the app's commands. Written by hand until tauri-specta
// generates this file; keep each one in step with `src-tauri/src/settings.rs`.

import { invoke } from "@tauri-apps/api/core";

/** The id Claude Code's login is kept under */
export const CLAUDE = "claude";

/** Keep `value` as `harness`'s login. A login goes in and never comes out. */
export function loginSet(harness: string, value: string): Promise<void> {
  return invoke("login_set", { harness, value });
}

/** Whether `harness` has a login: the only thing the app can learn about one. */
export function loginStatus(harness: string): Promise<boolean> {
  return invoke("login_status", { harness });
}

/** Forget `harness`'s login. */
export function loginClear(harness: string): Promise<void> {
  return invoke("login_clear", { harness });
}

/** One model or effort a harness lists for itself. */
export type Choice = {
  /** The value kelpie sends back to pick this one. */
  id: string;
  /** The label the harness gives it. */
  name: string;
  /** What the harness says about it, if it says anything. */
  description: string | null;
};

/** What a harness says it offers. */
export type Options = {
  models: Choice[];
  efforts: Choice[];
};

/** What `harness` offers: its own model and effort lists, kept until `refresh`. */
export function harnessOptions(harness: string, refresh: boolean): Promise<Options> {
  return invoke("harness_options", { harness, refresh });
}
