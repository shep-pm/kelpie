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
