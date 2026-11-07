import { Popover as PopoverPrimitive } from "@base-ui/react/popover";
import { cn } from "cn";

// docs/ui.md#尺寸 and #动效: a 16px-cornered panel that grows out of its button, scrolling when longer
// than the room on its side.

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
      <PopoverPrimitive.Positioner align={align} side={side} sideOffset={sideOffset} className="z-50">
        <PopoverPrimitive.Popup
          data-slot="popover-content"
          className={cn(
            "max-h-(--available-height) w-64 overflow-y-auto rounded-lg bg-background p-1.5 text-sm shadow-pop transition-[opacity,scale] duration-180 ease-out outline-none origin-(--transform-origin) data-ending-style:opacity-0 data-ending-style:duration-120 data-ending-style:ease-in data-starting-style:scale-96 data-starting-style:opacity-0",
            className,
          )}
          {...props}
        />
      </PopoverPrimitive.Positioner>
    </PopoverPrimitive.Portal>
  );
}

export { Popover, PopoverContent, PopoverTrigger };
