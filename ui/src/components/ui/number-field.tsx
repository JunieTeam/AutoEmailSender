import { type ComponentProps, useRef, useState } from "react";
import { composing } from "@/composing";
import { Input } from "@/components/ui/input";

/**
 * The number in what was typed, written as `String` writes numbers (very large ones with an
 * exponent), also as a Chinese input method types them (full width, "。"). null when nothing is
 * typed, NaN when it is no number.
 */
function parse(text: string) {
  const s = text.normalize("NFKC").replaceAll("。", ".").trim();
  if (s === "") return null;
  const n = /^-?(\d+\.?\d*|\.\d+)(e[+-]?\d+)?$/i.test(s) ? Number(s) : NaN;
  return Number.isFinite(n) ? n : NaN;
}

const format = (value: number | null) => (value === null ? "" : String(value));

/**
 * A text field for a number, without the browser's spin buttons (docs/ui.md#控件). Its value is the
 * number it shows, and null while it is empty or shows no number it takes: one out of `min`–`max`,
 * with more than `decimals` places, or text that is no number, which it marks, and Enter in it then
 * does not submit. Leaving it puts a number at the nearest one it takes, and turns text that is no
 * number back to the value it had; the arrow keys step it.
 */
export function NumberField({
  value,
  onValueChange,
  min = -Infinity,
  max = Infinity,
  decimals = 0,
  "aria-invalid": marked,
  ...input
}: {
  value: number | null;
  onValueChange: (value: number | null) => void;
  min?: number;
  max?: number;
  /** The decimal places it takes, none unless set. */
  decimals?: number;
} & Omit<
  ComponentProps<"input">,
  "value" | "onChange" | "onFocus" | "onBlur" | "onKeyDown" | "min" | "max" | "step" | "type" | "defaultValue"
>) {
  const [focused, setFocused] = useState(false);
  // The text and the value it stands for: while the field is focused the text is what was typed,
  // and otherwise it follows the value, which may stay as it was when the text gave none.
  const [shown, setShown] = useState({ text: format(value), value });
  if (!focused && value !== shown.value) setShown({ text: format(value), value });
  // The value as the field was focused, for text that is no number to go back to.
  const before = useRef(value);

  const round = (n: number) => Number(n.toFixed(decimals));
  const takes = (n: number) => n >= min && n <= max && round(n) === n;
  const nearest = (n: number) => Math.min(Math.max(round(n), min), max);
  const typed = parse(shown.text);
  const invalid = typed !== null && !takes(typed);

  function show(text: string, next: number | null) {
    if (next !== value) onValueChange(next);
    setShown({ text, value: next });
  }

  return (
    <Input
      {...input}
      inputMode={decimals > 0 ? "decimal" : "numeric"}
      autoComplete="off"
      spellCheck={false}
      value={shown.text}
      aria-invalid={invalid || marked}
      onFocus={() => {
        before.current = value;
        setFocused(true);
      }}
      onChange={(e) => {
        const n = parse(e.target.value);
        show(e.target.value, n !== null && takes(n) ? n : null);
      }}
      onBlur={() => {
        const next = typed === null ? null : Number.isNaN(typed) ? before.current : nearest(typed);
        show(format(next), next);
        setFocused(false);
      }}
      onKeyDown={(e) => {
        if (composing(e)) return;
        if (e.key === "Enter" && invalid) e.preventDefault();
        if (e.key === "ArrowUp" || e.key === "ArrowDown") {
          e.preventDefault();
          const next = nearest((typed === null || Number.isNaN(typed) ? 0 : typed) + (e.key === "ArrowUp" ? 1 : -1));
          show(format(next), next);
        }
      }}
    />
  );
}
