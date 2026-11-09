import { NumberField as NumberFieldPrimitive } from "@base-ui/react/number-field";
import { cn } from "cn";
import type { ComponentProps } from "react";
import { FIELD } from "@/components/ui/input";

/**
 * A text field for a number, without the browser's spin buttons (docs/ui.md#控件). What is typed is
 * kept between `min` and `max` and rounded to `decimals` places, whole numbers unless set; the
 * arrow keys step it. null is an empty field.
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
  return (
    <NumberFieldPrimitive.Root
      id={id}
      value={value}
      onValueChange={(v) => onValueChange(v)}
      onValueCommitted={onValueCommitted && ((v) => onValueCommitted(v))}
      min={min}
      max={max}
      format={{ useGrouping: false, maximumFractionDigits: decimals }}
      disabled={disabled}
      className="contents"
    >
      <NumberFieldPrimitive.Input className={cn("h-10", FIELD, className)} {...input} />
    </NumberFieldPrimitive.Root>
  );
}
