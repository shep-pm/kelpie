import { create } from "zustand";
import {
  commands,
  type Options,
  type ResolvedView,
  type Target,
  type Value,
} from "@/bindings";
import { askedAs } from "./harnesses";

/** What `harness_options` said for one harness. */
export type Offered =
  | { state: "loading" }
  | { state: "ready"; options: Options }
  | { state: "failed"; error: string };

type Settings = {
  /** The settings schema, `null` until read. */
  schema: Value | null;
  /** Every field of both tabs, `null` until read. */
  view: ResolvedView | null;
  /** The last failure to read or write, for the page to show. */
  error: string | null;
  /** What each harness offers, by the id a settings file holds. */
  offered: Record<string, Offered>;
  /** Read the schema, once, and every field with the layer it came from. */
  load: () => Promise<void>;
  /** Write `value` to one field of `layer`, then read the fields again. */
  set: (layer: Target, path: string, value: Value) => Promise<void>;
  /** Clear one field of `layer`, then read the fields again. */
  unset: (layer: Target, path: string) => Promise<void>;
  /** Ask for what `harness` offers; `refresh` asks the harness itself again. */
  loadOffered: (harness: string, refresh: boolean) => Promise<void>;
  /** Forget everything. For tests. */
  reset: () => void;
};

const empty = {
  schema: null,
  view: null,
  error: null,
  offered: {},
} satisfies Partial<Settings>;

export const useSettings = create<Settings>((set, get) => ({
  ...empty,

  async load() {
    try {
      const [schema, view] = await Promise.all([
        get().schema ?? commands.settingsSchema(),
        commands.settingsGet(null),
      ]);
      if (view.status === "error") set({ schema, error: view.error });
      else set({ schema, view: view.data, error: null });
    } catch (error) {
      set({ error: String(error) });
    }
  },

  async set(layer, path, value) {
    const done = await commands.settingsSet(layer, path, value);
    // A refused value leaves the form showing what the file still holds
    set({ error: done.status === "error" ? done.error : null });
    await get().load();
  },

  async unset(layer, path) {
    const done = await commands.settingsReset(layer, path);
    set({ error: done.status === "error" ? done.error : null });
    await get().load();
  },

  async loadOffered(harness, refresh) {
    const held = get().offered[harness];
    // A failed answer is asked again; a kept one only when `refresh` says so
    if (held?.state === "loading" || (held?.state === "ready" && !refresh)) {
      return;
    }
    // A refresh keeps the list in view while the harness answers
    if (held?.state !== "ready") {
      set((s) => ({
        offered: { ...s.offered, [harness]: { state: "loading" } },
      }));
    }
    let next: Offered;
    try {
      const asked = await commands.harnessOptions(askedAs(harness), refresh);
      next =
        asked.status === "ok"
          ? { state: "ready", options: asked.data }
          : { state: "failed", error: asked.error };
    } catch (error) {
      next = { state: "failed", error: String(error) };
    }
    set((s) => ({ offered: { ...s.offered, [harness]: next } }));
  },

  reset: () => set({ ...empty }),
}));
