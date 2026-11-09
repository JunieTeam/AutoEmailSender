// @vitest-environment happy-dom
import { act, type ReactNode, useEffect, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { formClose } from "@/components/form";
import { Dialog, DialogContent } from "./dialog";
import { NumberField } from "./number-field";

const roots: Root[] = [];

afterEach(() => {
  act(() => roots.splice(0).forEach((root) => root.unmount()));
  document.body.innerHTML = "";
});

/** Types `text` in the field in place of what is in it, as a user does. */
const typeIn = (input: HTMLInputElement, text: string) =>
  act(async () => {
    input.focus();
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, text);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });

/** Presses `key` in the field. */
const press = (input: HTMLInputElement, key: string) =>
  act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true })));

async function render(element: ReactNode) {
  const root = createRoot(document.body.appendChild(document.createElement("div")));
  roots.push(root);
  await act(async () => root.render(element));
  const input = document.querySelector<HTMLInputElement>('input[aria-label="数"]');
  if (!input) throw new Error("the field did not render");
  return input;
}

/**
 * A form with one number field; `seen` is called with each value the field gives, and `set` changes
 * the form's number from outside the field. With `keep`, the form keeps its number when the field
 * gives none, as the list of mentors keeps its days.
 */
async function mount(field: { min?: number; max?: number; decimals?: number; initial: number | null; keep?: boolean }) {
  const seen = vi.fn<(value: number | null) => void>();
  const submitted = vi.fn<(value: number | null) => void>();
  const outside: { set: (value: number | null) => void } = { set: () => {} };
  function Form() {
    const [value, setValue] = useState(field.initial);
    useEffect(() => {
      outside.set = setValue;
    }, []);
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
  const input = await render(<Form />);
  const form = input.form;
  if (!form) throw new Error("the field is not in its form");
  return {
    input,
    seen,
    submitted,
    type: (text: string) => typeIn(input, text),
    /** Presses Enter in the field, which submits its form as a browser does, unless the field takes the key. */
    enter: async () => {
      const key = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
      await act(async () => input.dispatchEvent(key));
      if (!key.defaultPrevented) await act(async () => form.requestSubmit());
    },
    focus: () => act(async () => input.focus()),
    leave: () => act(async () => input.blur()),
    set: (value: number | null) => act(async () => outside.set(value)),
    submit: () => act(async () => form.requestSubmit()),
  };
}

it("submits a price with decimals", async () => {
  const field = await mount({ min: 0, decimals: 6, initial: null });
  await field.type("0.25");
  await field.submit();
  expect(field.submitted).toHaveBeenCalledWith(0.25);
});

it("gives no number while what is typed is out of range, and the nearest one once left", async () => {
  const field = await mount({ min: 1, max: 65535, initial: 465 });
  await field.type("70000");
  expect(field.seen).toHaveBeenLastCalledWith(null);
  expect(field.input.getAttribute("aria-invalid")).toBe("true");
  await field.leave();
  expect(field.seen).toHaveBeenLastCalledWith(65535);
  expect(field.input.value).toBe("65535");
});

it("shows text that is no number as it is, and turns it back to the number once left", async () => {
  const field = await mount({ min: 0, decimals: 6, initial: 2 });
  await field.type("2中");
  expect(field.input.value).toBe("2中");
  expect(field.seen).toHaveBeenLastCalledWith(null);
  expect(field.input.getAttribute("aria-invalid")).toBe("true");
  await field.leave();
  expect(field.seen).toHaveBeenLastCalledWith(2);
  expect(field.input.value).toBe("2");
});

it("does not submit with Enter what it does not take", async () => {
  const field = await mount({ min: 0, decimals: 6, initial: 2 });
  await field.type("-0.25");
  await field.enter();
  expect(field.submitted).not.toHaveBeenCalled();
  await field.type("0.5");
  await field.enter();
  expect(field.submitted).toHaveBeenCalledWith(0.5);
});

it("shows the number kept for it again once left, without giving it again", async () => {
  const field = await mount({ min: 1, initial: 7, keep: true });
  await field.type("");
  expect(field.input.value).toBe("");
  await field.leave();
  expect(field.input.value).toBe("7");
  await field.type("+");
  await field.leave();
  expect(field.input.value).toBe("7");
  // Giving it again would start the list of mentors over at its first page.
  expect(field.seen).not.toHaveBeenCalledWith(7);
});

it("follows its number changed elsewhere until something is typed in it", async () => {
  const field = await mount({ min: 0, initial: 2 });
  await field.focus();
  await field.set(5);
  expect(field.input.value).toBe("5");
  await field.leave();
  expect(field.input.value).toBe("5");
  expect(field.seen).not.toHaveBeenCalled();
});

it("reads numbers as a Chinese input method types them", async () => {
  const field = await mount({ min: 0, decimals: 6, initial: null });
  await field.type("１２。５");
  expect(field.seen).toHaveBeenLastCalledWith(12.5);
  await field.leave();
  expect(field.input.value).toBe("12.5");
});

it("keeps its dialog open on Esc while it shows what it does not take", async () => {
  const close = vi.fn();
  function Priced() {
    const [price, setPrice] = useState<number | null>(null);
    return (
      <Dialog open onOpenChange={formClose(price !== null, false, close)}>
        <DialogContent>
          <NumberField aria-label="数" min={0} decimals={6} value={price} onValueChange={setPrice} />
        </DialogContent>
      </Dialog>
    );
  }
  const input = await render(<Priced />);
  // Too many places: the form has no price yet, so nothing else keeps the dialog open.
  await typeIn(input, "0.0000001");
  await press(input, "Escape");
  expect(close).not.toHaveBeenCalled();
  await typeIn(input, "");
  await press(input, "Escape");
  expect(close).toHaveBeenCalled();
});
