import { useSyncExternalStore } from "react";

/** docs/ui.md#布局: the window width where pages split in panes show one at a time (index.css `compact`). */
export const COMPACT = "(width <= 1000px)";

function subscribe(change: () => void) {
  const query = matchMedia(COMPACT);
  query.addEventListener("change", change);
  return () => query.removeEventListener("change", change);
}
const compact = () => matchMedia(COMPACT).matches;

/** Whether the window is as narrow as `COMPACT`, following it as it is resized. */
export function useCompact() {
  return useSyncExternalStore(subscribe, compact);
}
