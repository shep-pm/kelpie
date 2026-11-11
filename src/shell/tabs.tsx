import type { ComponentType } from "react";
import { create } from "zustand";

export type Tab = { id: string; title: string };

/** The PM's own tab: always first, and the only one that cannot close. */
export const PM_TAB: Tab = { id: "pm", title: "PM" };

type Tabs = {
  /** The PM first, then each opened tab in the order it opened. */
  tabs: Tab[];
  selected: string;
  /** Add `tab` and select it. A tab already open is only selected. */
  open: (tab: Tab) => void;
  /**
   * Hide the tab `id`. What the tab showed keeps running: a worker is not
   * stopped by closing its tab. The PM tab stays. Closing the selected tab
   * selects the one before it.
   */
  close: (id: string) => void;
  /** Show the open tab `id`. A tab that is not open is ignored. */
  select: (id: string) => void;
  /** Back to the PM tab alone. For tests. */
  reset: () => void;
};

const initial = { tabs: [PM_TAB], selected: PM_TAB.id };

export const useTabs = create<Tabs>()((set) => ({
  ...initial,
  open: (tab) =>
    set((s) =>
      s.tabs.some((t) => t.id === tab.id)
        ? { selected: tab.id }
        : { tabs: [...s.tabs, tab], selected: tab.id },
    ),
  close: (id) =>
    set((s) => {
      const at = s.tabs.findIndex((t) => t.id === id);
      if (id === PM_TAB.id || at < 0) return s;
      return {
        tabs: s.tabs.filter((t) => t.id !== id),
        selected:
          s.selected === id ? (s.tabs[at - 1]?.id ?? PM_TAB.id) : s.selected,
      };
    }),
  select: (id) =>
    set((s) => (s.tabs.some((t) => t.id === id) ? { selected: id } : s)),
  reset: () => set(initial),
}));

type Bodies = { bodies: Readonly<Record<string, ComponentType>> };

const useBodies = create<Bodies>()(() => ({ bodies: {} }));

/**
 * Say what the tab `id` shows. Returns the call that takes it back.
 *
 * `Body` renders as a component of its own, so it can use hooks. A tab
 * nothing is registered for shows an empty body, and shows `Body` as soon as
 * it is registered, whether or not the tab is open yet.
 */
export function registerTabBody(id: string, Body: ComponentType) {
  useBodies.setState((s) => ({ bodies: { ...s.bodies, [id]: Body } }));
  return () =>
    useBodies.setState((s) => {
      if (s.bodies[id] !== Body) return s;
      const { [id]: _taken, ...rest } = s.bodies;
      return { bodies: rest };
    });
}

/** The body of the tab `id`: what is registered for it, and nothing before. */
export function TabBody({ id }: { id: string }) {
  const Body = useBodies((s) => s.bodies[id]);
  return Body ? <Body /> : null;
}
