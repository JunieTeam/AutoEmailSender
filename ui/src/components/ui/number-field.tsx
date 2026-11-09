import { NumberField as NumberFieldPrimitive } from "@base-ui/react/number-field";
import { cn } from "cn";
import { type ComponentProps, useReducer } from "react";
import { FIELD } from "@/components/ui/input";

/**
 * A text field for a number, without the browser's spin buttons (docs/ui.md#控件). The value is the
 * number the field shows, rounded to `decimals` places, whole numbers unless set; it is null while
 * the field is empty or shows no number between `min` and `max`. Leaving the field puts a number
 * out of range at the nearer end and clears text that is no number; the arrow keys step it.
 */
export function NumberField({
  value,
  onValueChange,
  onValueCommitted,
  min,
  max,
  decimals = 0,
  id,
  className,
  disabled,
  ...input
}: {
  value: number | null;
  onValueChange: (value: number | null) => void;
  /** Called as the field is left after typing, and with each step of the arrow keys. */
  onValueCommitted?: (value: number | null) => void;
  min?: number;
  max?: number;
  decimals?: number;
} & Omit<ComponentProps<"input">, "value" | "onChange" | "min" | "max" | "step" | "type" | "defaultValue">) {
  const nearest = (v: number) => Math.min(Math.max(v, min ?? -Infinity), max ?? Infinity);
  // Base UI shows the value again only when it renders after the field is left, and text that is
  // no number changes nothing that would render it.
  const [, render] = useReducer((n: number) => n + 1, 0);
  return (
    <NumberFieldPrimitive.Root
      id={id}
      value={value}
      // Base UI would otherwise give the nearest number in range while the field still shows what
      // was typed, and a form could save a number the user never saw.
      allowOutOfRange
      onValueChange={(v) => onValueChange(v !== null && nearest(v) === v ? v : null)}
      onValueCommitted={(v) => {
        const kept = v === null ? null : nearest(v);
        if (kept !== v) onValueChange(kept);
        onValueCommitted?.(kept);
      }}
      min={min}
      max={max}
      // The form checks the number against the step: whole numbers unless any.
      step={decimals > 0 ? "any" : 1}
      format={{ useGrouping: false, maximumFractionDigits: decimals }}
      disabled={disabled}
      className="contents"
    >
      <NumberFieldPrimitive.Input
        className={cn("h-10", FIELD, className)}
        // Runs before Base UI's handler, which then gives the number where the text has one.
        onChange={() => onValueChange(null)}
        onBlur={render}
        {...input}
      />
    </NumberFieldPrimitive.Root>
  );
}
