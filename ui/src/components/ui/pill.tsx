import { cn } from "cn";
import { ChevronDownIcon } from "lucide-react";
import { SelectRoot, SelectTrigger } from "@/components/ui/select";

/**
 * docs/ui.md#控件: a filter in a row of them, or the sort beside them, is a pill saying what is
 * chosen, marked while it narrows the list.
 */
export function pillClass(active: boolean) {
  return cn(
    "flex h-8 shrink-0 items-center gap-1 rounded-full px-3 text-sm whitespace-nowrap",
    active ? "bg-selected font-medium text-brand-text" : "bg-muted text-muted-foreground hover:text-foreground",
  );
}

/** The pill's words: its label, then what is chosen, if anything. */
export function PillLabel({ label, shown }: { label: string; shown: string | undefined }) {
  return (
    <>
      <span className="max-w-60 truncate">{shown === undefined ? label : `${label}：${shown}`}</span>
      <ChevronDownIcon className="size-3.5" strokeWidth={1.6} aria-hidden />
    </>
  );
}

/** One of a few choices, written out unless it is undefined, which is no choice. */
export function PillSelect<T>({
  label,
  options,
  value,
  active,
  onChange,
  className,
}: {
  label: string;
  options: readonly (readonly [NoInfer<T>, string])[];
  value: T;
  active: boolean;
  onChange: (value: NoInfer<T>) => void;
  className?: string;
}) {
  const shown = value === undefined ? undefined : options.find(([o]) => o === value)?.[1];
  return (
    <SelectRoot value={value} options={options} onChange={onChange}>
      <SelectTrigger className={cn(pillClass(active), className)}>
        <PillLabel label={label} shown={shown} />
      </SelectTrigger>
    </SelectRoot>
  );
}
