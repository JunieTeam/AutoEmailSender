import { Tooltip as TooltipPrimitive } from "@base-ui/react/tooltip";
import { cn } from "cn";
import type { ReactElement } from "react";
import { POPUP } from "@/components/ui/popover";

/**
 * A hint shown while the pointer rests on `children`, drawn like our menus rather than the system's
 * (docs/ui.md#控件); without `content`, none. Nothing only a hint says may be needed to use the app.
 */
/** Once a hint has shown, the next one under the pointer shows at once, as the system's do. */
export const TooltipProvider = TooltipPrimitive.Provider;

export function Tooltip({ content, children }: { content: string | undefined; children: ReactElement }) {
  if (content === undefined) return children;
  return (
    <TooltipPrimitive.Root>
      <TooltipPrimitive.Trigger render={children} />
      <TooltipPrimitive.Portal>
        <TooltipPrimitive.Positioner sideOffset={6} className="z-50">
          <TooltipPrimitive.Popup
            className={cn(POPUP, "max-w-[min(360px,var(--available-width))] px-2.5 py-1.5 text-xs wrap-anywhere")}
          >
            {content}
          </TooltipPrimitive.Popup>
        </TooltipPrimitive.Positioner>
      </TooltipPrimitive.Portal>
    </TooltipPrimitive.Root>
  );
}
