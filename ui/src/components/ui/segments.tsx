import { Tabs } from "@base-ui/react/tabs";
import type { ReactNode } from "react";
import { cn } from "cn";

/**
 * Pills that pick one of a few values, such as a page's tab, a list's filter or a time range; the
 * white block slides to the chosen one (docs/ui.md#控件). Base UI gets each option's position, so a
 * value may be anything, `undefined` included.
 */
export function Segments<T>({
  label,
  value,
  options,
  onChange,
  disabled,
  className,
}: {
  label: string;
  value: T;
  // The type comes from `value`, so options written in place keep their literal types.
  options: readonly (readonly [NoInfer<T>, ReactNode])[];
  onChange: (value: NoInfer<T>) => void;
  disabled?: boolean;
  className?: string;
}) {
  return (
    <Tabs.Root
      // The list lays out as if it were the root, wherever it sits.
      className="contents"
      value={options.findIndex(([option]) => option === value)}
      onValueChange={(i: number) => {
        const chosen = options[i];
        if (chosen) onChange(chosen[0]);
      }}
    >
      <Tabs.List
        aria-label={label}
        className={cn(
          "relative z-0 inline-flex max-w-full gap-0.5 overflow-x-auto rounded-full bg-muted p-[3px] [scrollbar-width:none]",
          className,
        )}
      >
        {options.map(([, text], i) => (
          <Tabs.Tab
            // The options are a fixed list.
            key={i}
            value={i}
            disabled={disabled}
            className="flex h-[30px] shrink-0 items-center justify-center gap-1.5 rounded-full px-3.5 text-sm whitespace-nowrap text-muted-foreground transition-colors hover:text-foreground data-active:font-medium data-active:text-foreground phone:h-10"
          >
            {text}
          </Tabs.Tab>
        ))}
        <Tabs.Indicator className="absolute top-(--active-tab-top) left-(--active-tab-left) -z-10 h-(--active-tab-height) w-(--active-tab-width) rounded-full bg-background shadow-sm transition-[left,width] duration-180 ease-out dark:bg-input" />
      </Tabs.List>
    </Tabs.Root>
  );
}
