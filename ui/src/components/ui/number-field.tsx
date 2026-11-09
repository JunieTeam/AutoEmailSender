import { type ComponentProps, useState } from "react";
import { composing } from "@/composing";
import { Input } from "@/components/ui/input";

/** The largest numbers the core takes as 16- and 32-bit whole numbers (`u16`, `u32`). */
export const U16 = 65535;
export const U32 = 4294967295;

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
 * A text field for a number, without the browser's spin buttons (docs/ui.md#控件). It shows its
 * value until something is typed in it, and then what was typed until it is left. Its value is the
 * number typed, and null while that is empty or no number it takes: one out of `min`–`max`, with
 * more than `decimals` places, or text that is no number, which it marks, and in which Enter does not
 * submit nor Esc close the form, so the text is not lost. Leaving it puts a number at the nearest one
 * it takes, and turns text that is no number back to the value it had; the arrow keys step it.
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
  "value" | "onChange" | "onBlur" | "onKeyDown" | "min" | "max" | "step" | "type" | "defaultValue"
>) {
  // What was typed since the field was last left, and the value it had before, for text that is no
  // number to go back to.
  const [draft, setDraft] = useState<{ text: string; from: number | null } | null>(null);
  const text = draft ? draft.text : format(value);

  const round = (n: number) => Number(n.toFixed(decimals));
  const takes = (n: number) => n >= min && n <= max && round(n) === n;
  const nearest = (n: number) => Math.min(Math.max(round(n), min), max);
  const typed = parse(text);
  const invalid = typed !== null && !takes(typed);

  function give(next: number | null) {
    if (next !== value) onValueChange(next);
  }

  function type(next: string) {
    const n = parse(next);
    give(n !== null && takes(n) ? n : null);
    setDraft({ text: next, from: draft ? draft.from : value });
  }

  return (
    <Input
      {...input}
      inputMode={decimals > 0 ? "decimal" : "numeric"}
      autoComplete="off"
      spellCheck={false}
      value={text}
      aria-invalid={invalid || marked}
      onChange={(e) => type(e.target.value)}
      onBlur={() => {
        if (!draft) return;
        give(typed === null ? null : Number.isNaN(typed) ? draft.from : nearest(typed));
        setDraft(null);
      }}
      onKeyDown={(e) => {
        if (composing(e)) return;
        if ((e.key === "Enter" || e.key === "Escape") && invalid) {
          // Base UI closes a dialog on Esc even when its default is prevented; its own combobox keeps
          // an Esc it uses from the dialog the same way.
          e.preventDefault();
          e.stopPropagation();
        }
        if (e.key === "ArrowUp" || e.key === "ArrowDown") {
          e.preventDefault();
          const start = typed === null || Number.isNaN(typed) ? 0 : typed;
          type(format(nearest(start + (e.key === "ArrowUp" ? 1 : -1))));
        }
      }}
    />
  );
}
