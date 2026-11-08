import { useRouterState } from "@tanstack/react-router";
import { useLayoutEffect, useRef } from "react";

const positions = new Map<string, number>();

/**
 * Keeps where a list was scrolled with its place in the history: going back to it, say from a
 * mentor's page, returns to the same row, and another page of it or other filters start at the
 * top. Applied once the list has rows to scroll. Not the router's own scroll restoration, which
 * also hands positions on to whatever sits at the same place in the next page.
 */
export function useKeptScroll<T extends HTMLElement>(ready: boolean) {
  // The location the rows on show belong to: while a navigation is under way, the router's
  // location is already the next one, and a position restored then would be cut to the old rows.
  const key = useRouterState({
    select: (s) => {
      const shown = s.resolvedLocation ?? s.location;
      return shown.state.key ?? shown.href;
    },
  });
  const ref = useRef<T>(null);
  useLayoutEffect(() => {
    const list = ref.current;
    if (!list || !ready) return undefined;
    list.scrollTop = positions.get(key) ?? 0;
    return () => {
      positions.set(key, list.scrollTop);
    };
  }, [key, ready]);
  return ref;
}
