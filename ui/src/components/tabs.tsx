import { Activity, useState } from "react";

/**
 * docs/ui.md#交互: the views behind a segmented control, each kept as it was while another is on
 * show, so that coming back finds it scrolled, expanded and ticked as it was left. A view is first
 * drawn when first shown; showing one again fades it in as a new one does, the animation starting
 * over once it is no longer hidden.
 */
export function Tabs<T extends string | undefined>({
  shown,
  children,
}: {
  shown: T;
  children: (tab: T) => React.ReactNode;
}) {
  const [seen, setSeen] = useState<T[]>([shown]);
  if (!seen.includes(shown)) setSeen([...seen, shown]);
  return seen.map((tab) => (
    <Activity key={tab ?? ""} mode={tab === shown ? "visible" : "hidden"}>
      <div className="flex min-h-0 flex-1 animate-enter flex-col">{children(tab)}</div>
    </Activity>
  ));
}
