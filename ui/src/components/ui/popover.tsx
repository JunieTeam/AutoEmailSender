import { Popover as PopoverPrimitive } from "@base-ui/react/popover";
import { cn } from "cn";
import { titleBar } from "@/host";

// docs/ui.md#尺寸 and #动效: a 16px-cornered panel that grows out of its button, scrolling when longer
// than the room on its side.

/**
 * How far popups keep from the window's edges: Base UI's 5px, and clear of the macOS window's
 * buttons.
 */
export const COLLISION_PADDING = { top: titleBar + 5, right: 5, bottom: 5, left: 5 };

/** The floating panel of popovers, menus and lists of choices. */
export const POPUP =
  "rounded-lg bg-background p-1.5 text-sm shadow-pop outline-none origin-(--transform-origin) transition-[opacity,scale] duration-180 ease-out data-ending-style:opacity-0 data-ending-style:duration-120 data-ending-style:ease-in data-starting-style:scale-96 data-starting-style:opacity-0";

/**
 * docs/ui.md#控件: a menu is as wide as its widest entry and no narrower than its button. Panels of
 * prose set a width of their own.
 */
export const FIT = "w-max min-w-(--anchor-width) max-w-[min(360px,var(--available-width))]";

/** The scrolling list of choices in a popup. */
export const LIST = "max-h-[min(var(--available-height),360px)] overflow-y-auto p-1.5 outline-none";

/** A choice in a list, highlighted by the pointer or the arrow keys. */
export const OPTION =
  "flex items-center rounded-md px-3 py-2 wrap-anywhere outline-none select-none data-disabled:opacity-40 data-highlighted:bg-muted";

/** An entry of a menu: an action, or a choice. */
export const MENU_ITEM =
  "flex w-full items-center gap-2.5 rounded-md px-3 py-2 text-left wrap-anywhere hover:bg-muted disabled:opacity-40 disabled:hover:bg-transparent";

const Popover = PopoverPrimitive.Root;

const PopoverTrigger = PopoverPrimitive.Trigger;

function PopoverContent({
  className,
  align = "center",
  side = "bottom",
  sideOffset = 6,
  ...props
}: PopoverPrimitive.Popup.Props & Pick<PopoverPrimitive.Positioner.Props, "align" | "side" | "sideOffset">) {
  return (
    <PopoverPrimitive.Portal>
      <PopoverPrimitive.Positioner
        align={align}
        side={side}
        sideOffset={sideOffset}
        collisionPadding={COLLISION_PADDING}
        className="z-50"
      >
        <PopoverPrimitive.Popup
          data-slot="popover-content"
          className={cn("max-h-(--available-height) overflow-y-auto", FIT, POPUP, className)}
          {...props}
        />
      </PopoverPrimitive.Positioner>
    </PopoverPrimitive.Portal>
  );
}

export { Popover, PopoverContent, PopoverTrigger };
