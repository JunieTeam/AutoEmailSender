import type { UseQueryResult } from "@tanstack/react-query";
import { useRouterState } from "@tanstack/react-router";
import { useLayoutEffect, useRef } from "react";

const positions = new Map<string, number>();
// The last position of each list on each page, for a return from the sidebar, which is a step of
// its own in the history.
const last = new Map<string, number>();

/**
 * Keeps where a list was scrolled with its place in the history: going back to it, say from a
 * mentor's page, returns to the same row. A list showing something else, another page or other
 * filters, starts at the top; a step in the history that leaves what it shows as it was, such as
 * opening a conversation beside it, leaves it where it is, as does showing again a tab kept while
 * another was on show; returning to the page from the sidebar goes back to where it was last. `view` says what the list shows when the address holds
 * more than that; otherwise the address does. Applied once `rows` has the rows of that, not those
 * of before kept on show while they load.
 *
 * Not the router's own scroll restoration, which also hands positions on to whatever sits at the
 * same place in the next page.
 */
export function useKeptScroll<T extends HTMLElement>(
  rows: Pick<UseQueryResult, "isSuccess" | "isPlaceholderData">,
  view?: string,
) {
  // The location the rows on show belong to: while a navigation is under way, the router's
  // location is already the next one, and a position restored then would be cut to the old rows.
  const entry = useRouterState({
    select: (s) => {
      const shown = s.resolvedLocation ?? s.location;
      return shown.state.key ?? shown.href;
    },
  });
  const href = useRouterState({ select: (s) => (s.resolvedLocation ?? s.location).href });
  const page = useRouterState({ select: (s) => (s.resolvedLocation ?? s.location).pathname });
  const returning = useRouterState({ select: (s) => (s.resolvedLocation ?? s.location).state.returning === true });
  const shows = view ?? href;
  const ready = rows.isSuccess && !rows.isPlaceholderData;
  const ref = useRef<T>(null);
  const showed = useRef(shows);
  // Where this list was when its effect last ended: a list in a tab kept while another is on show
  // loses its position in the browser, as everything hidden does, and gets it back from here.
  const kept = useRef<number>(undefined);
  useLayoutEffect(() => {
    const list = ref.current;
    if (!list || !ready) return undefined;
    const at = `${entry} ${shows}`;
    const left = `${page} ${shows}`;
    const saved =
      positions.get(at) ??
      (returning ? last.get(left) : undefined) ??
      (showed.current === shows ? kept.current : undefined);
    if (saved !== undefined) list.scrollTop = saved;
    else if (showed.current !== shows) list.scrollTop = 0;
    showed.current = shows;
    return () => {
      positions.set(at, list.scrollTop);
      last.set(left, list.scrollTop);
      kept.current = list.scrollTop;
    };
  }, [entry, page, shows, returning, ready]);
  return ref;
}
