// @vitest-environment happy-dom
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { NumberField } from "./number-field";

/**
 * A form with one number field; `seen` is called with each value the field gives. With `keep`, the
 * form keeps its number when the field gives none, as the list of mentors keeps its days.
 */
async function mount(field: { min?: number; max?: number; decimals?: number; initial: number | null; keep?: boolean }) {
  const seen = vi.fn<(value: number | null) => void>();
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
            if (v !== null || !field.keep) setValue(v);
          }}
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
    submitted,
    type: (text: string) =>
      act(async () => {
        input.focus();
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, text);
        input.dispatchEvent(new Event("input", { bubbles: true }));
      }),
    /** Presses Enter in the field, which submits its form as a browser does, unless the field takes the key. */
    enter: () =>
      act(async () => {
        const key = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
        input.dispatchEvent(key);
        if (!key.defaultPrevented) form.requestSubmit();
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
    expect(field.input.getAttribute("aria-invalid")).toBe("true");
    await field.leave();
    expect(field.seen).toHaveBeenLastCalledWith(65535);
    expect(field.input.value).toBe("65535");
  } finally {
    field.unmount();
  }
});

it("shows text that is no number as it is, and turns it back to the number once left", async () => {
  const field = await mount({ min: 0, decimals: 6, initial: 2 });
  try {
    await field.type("2中");
    expect(field.input.value).toBe("2中");
    expect(field.seen).toHaveBeenLastCalledWith(null);
    expect(field.input.getAttribute("aria-invalid")).toBe("true");
    await field.leave();
    expect(field.seen).toHaveBeenLastCalledWith(2);
    expect(field.input.value).toBe("2");
  } finally {
    field.unmount();
  }
});

it("does not submit with Enter what it does not take", async () => {
  const field = await mount({ min: 0, decimals: 6, initial: 2 });
  try {
    await field.type("-0.25");
    await field.enter();
    expect(field.submitted).not.toHaveBeenCalled();
    await field.type("0.5");
    await field.enter();
    expect(field.submitted).toHaveBeenCalledWith(0.5);
  } finally {
    field.unmount();
  }
});

it("shows the number kept for it again once left, without giving it again", async () => {
  const field = await mount({ min: 1, initial: 7, keep: true });
  try {
    await field.type("");
    expect(field.input.value).toBe("");
    await field.leave();
    expect(field.input.value).toBe("7");
    await field.type("+");
    await field.leave();
    expect(field.input.value).toBe("7");
    // Giving it again would start the list of mentors over at its first page.
    expect(field.seen).not.toHaveBeenCalledWith(7);
  } finally {
    field.unmount();
  }
});

it("reads numbers as a Chinese input method types them", async () => {
  const field = await mount({ min: 0, decimals: 6, initial: null });
  try {
    await field.type("１２。５");
    expect(field.seen).toHaveBeenLastCalledWith(12.5);
    await field.leave();
    expect(field.input.value).toBe("12.5");
  } finally {
    field.unmount();
  }
});
