import { useState } from "react";
import { Select } from "@/components/ui/select";

/** The times of a day a quarter of an hour apart, as "HH:MM". */
const QUARTERS = Array.from(
  { length: 96 },
  (_, i) => `${String(Math.floor(i / 4)).padStart(2, "0")}:${String((i % 4) * 15).padStart(2, "0")}`,
);

/**
 * A time of day as "HH:MM", a quarter of an hour apart (docs/ui.md#控件). A time set otherwise, such
 * as by an Agent or before, stays among them while the field is open, to choose again after another.
 */
export function TimeSelect({
  value,
  onChange,
  none,
  ...field
}: {
  value: string;
  onChange: (value: string) => void;
  /** What no time is called, shown first, when the time may be left out. */
  none?: string;
  id?: string;
  className?: string;
  disabled?: boolean;
  "aria-label"?: string;
}) {
  const [first] = useState(value);
  const times = [...new Set([...QUARTERS, first, value])].filter((t) => t !== "").toSorted();
  const options = [...(none === undefined ? [] : [["", none] as const]), ...times.map((t) => [t, t] as const)];
  return <Select value={value} options={options} onChange={onChange} {...field} />;
}
