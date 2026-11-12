import { CircleQuestionMark } from "lucide-react";
import type { ReactNode } from "react";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";

/**
 * docs/ui.md#控件: the "?" after a title, whose details show while the pointer rests on it or after a
 * click; the line under the title says what the thing is.
 */
export function Help({ label, children }: { label: string; children: ReactNode }) {
  return (
    <Popover>
      <PopoverTrigger
        openOnHover
        aria-label={`“${label}”的说明`}
        className="grid size-6 shrink-0 place-items-center rounded-full text-faint hover:text-muted-foreground data-popup-open:text-muted-foreground"
      >
        <CircleQuestionMark className="size-4" strokeWidth={1.8} />
      </PopoverTrigger>
      <PopoverContent align="start" className="w-80 space-y-2 p-3.5 leading-relaxed">
        {children}
      </PopoverContent>
    </Popover>
  );
}
