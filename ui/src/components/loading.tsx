import { LoaderCircle } from "lucide-react";

/** docs/ui.md#动效: a spinner in the content area, shown only once the data takes longer than 300ms. */
export function Loading() {
  return (
    <div role="status" aria-label="正在加载" className="grid min-h-24 flex-1 animate-appear place-items-center">
      <LoaderCircle className="size-6 animate-spin text-faint" strokeWidth={1.6} />
    </div>
  );
}
