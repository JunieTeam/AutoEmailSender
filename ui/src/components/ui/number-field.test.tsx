// @vitest-environment happy-dom
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { NumberField } from "./number-field";

/** A form with one number field; `seen` is called with each value the field gives, `left` as it is left. */
async function mount(field: { min?: number; max?: number; decimals?: number; initial: number | null }) {
  const seen = vi.fn<(value: number | null) => void>();
  const left = vi.fn<(value: number | null) => void>();
  const submitted = vi.fn<(value: number | null) => void>();
  function Form() {
    const [value, setValue] = useState(field.initial);
    return (
      <form
        onSubmit={(e) => {
          e.preventDefault();
          submitted(value);
        }}
      >
        <NumberField
          aria-label="数"
          min={field.min}
          max={field.max}
          decimals={field.decimals}
          value={value}
          onValueChange={(v) => {
            seen(v);
            setValue(v);
          }}
          onValueCommitted={left}
        />
        <button type="submit">保存</button>
      </form>
    );
  }
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<Form />));
  const input = container.querySelector<HTMLInputElement>('input[aria-label="数"]');
  const form = container.querySelector("form");
  if (!input || !form) throw new Error("the field did not render");
  return {
    input,
    seen,
    left,
    submitted,
    type: (text: string) =>
      act(async () => {
        input.focus();
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, text);
        input.dispatchEvent(new Event("input", { bubbles: true }));
      }),
    leave: () => act(async () => input.blur()),
    submit: () => act(async () => form.requestSubmit()),
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

it("submits a price with decimals", async () => {
  const field = await mount({ min: 0, decimals: 6, initial: null });
  try {
    await field.type("0.25");
    await field.submit();
    expect(field.submitted).toHaveBeenCalledWith(0.25);
  } finally {
    field.unmount();
  }
});

it("gives no number while what is typed is out of range, and the nearest one once left", async () => {
  const field = await mount({ min: 1, max: 65535, initial: 465 });
  try {
    await field.type("70000");
    expect(field.seen).toHaveBeenLastCalledWith(null);
    await field.leave();
    expect(field.seen).toHaveBeenLastCalledWith(65535);
    expect(field.input.value).toBe("65535");
  } finally {
    field.unmount();
  }
});

it("takes text that is no number for an emptied field", async () => {
  const field = await mount({ min: 0, decimals: 6, initial: 2 });
  try {
    await field.type("+");
    expect(field.seen).toHaveBeenLastCalledWith(null);
    await field.leave();
    expect(field.left).toHaveBeenLastCalledWith(null);
    expect(field.input.value).toBe("");
  } finally {
    field.unmount();
  }
});
