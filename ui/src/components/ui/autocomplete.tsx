import { Autocomplete as AutocompletePrimitive } from "@base-ui/react/autocomplete";
import { cn } from "cn";
import type { ComponentProps } from "react";
import { FIELD } from "@/components/ui/input";
import { COLLISION_PADDING, FIT, LIST, OPTION, POPUP } from "@/components/ui/popover";

/**
 * A text field suggesting values that match what is typed, in a list of our own (docs/ui.md#控件);
 * other text may be typed too.
 */
export function Autocomplete({
  value,
  onValueChange,
  items,
  className,
  ...input
}: {
  value: string;
  onValueChange: (value: string) => void;
  items: string[];
} & Omit<ComponentProps<"input">, "value" | "onChange" | "defaultValue">) {
  return (
    <AutocompletePrimitive.Root items={items} value={value} onValueChange={(v) => onValueChange(v)} openOnInputClick>
      <AutocompletePrimitive.Input className={cn("h-10", FIELD, className)} {...input} />
      <AutocompletePrimitive.Portal>
        <AutocompletePrimitive.Positioner
          align="start"
          sideOffset={6}
          collisionPadding={COLLISION_PADDING}
          className="z-50 data-empty:hidden"
        >
          <AutocompletePrimitive.Popup className={cn(FIT, POPUP, "p-0")}>
            <AutocompletePrimitive.List className={LIST}>
              {(item: string) => (
                <AutocompletePrimitive.Item key={item} value={item} className={OPTION}>
                  {item}
                </AutocompletePrimitive.Item>
              )}
            </AutocompletePrimitive.List>
          </AutocompletePrimitive.Popup>
        </AutocompletePrimitive.Positioner>
      </AutocompletePrimitive.Portal>
    </AutocompletePrimitive.Root>
  );
}
