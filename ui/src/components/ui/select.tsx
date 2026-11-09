import { Select as SelectPrimitive } from "@base-ui/react/select";
import { cn } from "cn";
import { ChevronDownIcon } from "lucide-react";
import type { ReactNode, Ref } from "react";
import { FIELD } from "@/components/ui/input";
import { FIT, LIST, OPTION, POPUP } from "@/components/ui/popover";

/** The choices, each a value and how it is shown. */
export type Options<T> = readonly (readonly [T, ReactNode])[];

/**
 * Picks one of a few values from a list of our own, not the system's (docs/ui.md#控件). Base UI
 * gets each option's position, so a value may be anything, `undefined` included.
 */
export function SelectRoot<T>({
  value,
  options,
  onChange,
  disabled,
  children,
}: {
  value: T;
  options: Options<NoInfer<T>>;
  onChange: (value: NoInfer<T>) => void;
  disabled?: boolean;
  children: ReactNode;
}) {
  const index = options.findIndex(([option]) => option === value);
  return (
    <SelectPrimitive.Root
      items={options.map(([, label], i) => ({ value: i, label }))}
      value={index === -1 ? null : index}
      onValueChange={(i: number | null) => {
        const chosen = i === null ? undefined : options[i];
        if (chosen) onChange(chosen[0]);
      }}
      disabled={disabled}
    >
      {children}
      <SelectPrimitive.Portal>
        <SelectPrimitive.Positioner alignItemWithTrigger={false} align="start" sideOffset={6} className="z-50">
          <SelectPrimitive.Popup className={cn(FIT, POPUP, "p-0")}>
            <SelectPrimitive.List className={LIST}>
              {options.map(([, label], i) => (
                <SelectPrimitive.Item
                  // The options are a fixed list.
                  key={i}
                  value={i}
                  className={cn(OPTION, "data-selected:font-medium data-selected:text-brand-text")}
                >
                  <SelectPrimitive.ItemText>{label}</SelectPrimitive.ItemText>
                </SelectPrimitive.Item>
              ))}
            </SelectPrimitive.List>
          </SelectPrimitive.Popup>
        </SelectPrimitive.Positioner>
      </SelectPrimitive.Portal>
    </SelectPrimitive.Root>
  );
}

export const SelectTrigger = SelectPrimitive.Trigger;

/** A form field choosing one of the options; `placeholder` shows while none is chosen. */
export function Select<T>({
  value,
  options,
  onChange,
  disabled,
  placeholder,
  id,
  className,
  ref,
  "aria-label": label,
}: {
  value: T;
  options: Options<NoInfer<T>>;
  onChange: (value: NoInfer<T>) => void;
  disabled?: boolean;
  placeholder?: string;
  id?: string;
  className?: string;
  ref?: Ref<HTMLButtonElement>;
  "aria-label"?: string;
}) {
  return (
    <SelectRoot value={value} options={options} onChange={onChange} disabled={disabled}>
      <SelectPrimitive.Trigger
        ref={ref}
        id={id}
        aria-label={label}
        className={cn(
          "flex h-10 items-center justify-between gap-2 text-left data-disabled:opacity-40 data-popup-open:ring-1 data-popup-open:ring-primary data-popup-open:ring-inset",
          FIELD,
          className,
        )}
      >
        <SelectPrimitive.Value placeholder={placeholder} className="min-w-0 truncate data-placeholder:text-faint" />
        <ChevronDownIcon className="size-4 shrink-0 text-muted-foreground" strokeWidth={1.6} aria-hidden />
      </SelectPrimitive.Trigger>
    </SelectRoot>
  );
}
