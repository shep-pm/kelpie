import { useSyncExternalStore } from "react";

/** The class `src/main.tsx` puts on `<html>` when the window is dark. */
const DARK = "dark";

function subscribe(onChange: () => void): () => void {
  const watch = new MutationObserver(onChange);
  watch.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["class"],
  });
  return () => watch.disconnect();
}

const snapshot = () => document.documentElement.classList.contains(DARK);

/** Whether the window is dark: kelpie's stand-in for chelone's `useResolvedTheme`. */
export function useDark(): boolean {
  return useSyncExternalStore(subscribe, snapshot, () => false);
}
