/**
 * docs/ui.md#快捷键: the key a press gives the page, in lower case; none while typing, with a
 * modifier held, or with a dialog open, which keeps its keys to itself.
 */
export function pageKey(e: KeyboardEvent): string | undefined {
  if (
    e.defaultPrevented ||
    e.metaKey ||
    e.ctrlKey ||
    e.altKey ||
    (e.target instanceof Element && e.target.closest("input,textarea,select,[contenteditable=true],[role=dialog]")) ||
    document.querySelector('[role="dialog"]')
  )
    return undefined;
  return e.key.toLowerCase();
}
