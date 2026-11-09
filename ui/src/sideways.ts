/**
 * docs/ui.md#控件: a ref for a row that scrolls sideways, which then also follows a mouse wheel, as
 * it only turns up and down. Trackpads and touch scroll it themselves; past either end, the wheel
 * scrolls the page.
 */
export function scrollsSideways(row: HTMLElement | null) {
  // React calls the cleanup returned below in place of the ref with null.
  if (!row) return undefined;
  const wheel = (e: WheelEvent) => {
    if (e.deltaX !== 0 || e.ctrlKey) return;
    const end = row.scrollWidth - row.clientWidth;
    if ((e.deltaY < 0 && row.scrollLeft <= 0) || (e.deltaY > 0 && row.scrollLeft >= end)) return;
    e.preventDefault();
    row.scrollLeft += e.deltaY;
  };
  row.addEventListener("wheel", wheel, { passive: false });
  return () => row.removeEventListener("wheel", wheel);
}
