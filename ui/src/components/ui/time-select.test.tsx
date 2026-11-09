// @vitest-environment happy-dom
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { TimeSelect } from "./time-select";

/** Opens the list and picks the option so named, as a click does. */
async function choose(option: string) {
  await act(async () => document.querySelector<HTMLElement>("#time")?.click());
  const item = await vi.waitFor(() => {
    const found = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find((o) => o.textContent === option);
    if (!found) throw new Error(`no option ${option}`);
    return found;
  });
  await act(async () => {
    item.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    item.click();
  });
}

it("keeps a time off the quarter hours to choose again after another", async () => {
  const chosen = vi.fn<(time: string) => void>();
  function Field() {
    const [time, setTime] = useState("09:07");
    return (
      <TimeSelect
        id="time"
        value={time}
        onChange={(t) => {
          chosen(t);
          setTime(t);
        }}
      />
    );
  }
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => root.render(<Field />));
    await choose("09:15");
    await choose("09:07");
    expect(chosen.mock.calls).toEqual([["09:15"], ["09:07"]]);
  } finally {
    act(() => root.unmount());
    container.remove();
  }
});
