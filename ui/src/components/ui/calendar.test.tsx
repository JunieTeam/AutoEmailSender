// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { CallError, Holidays } from "@/api/types";
import { Calendar } from "./calendar";

const remote = vi.hoisted(() => ({ failures: 0, answered: Promise.resolve() }));
vi.mock("@/api/call", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/api/call")>()),
  call: async (op: string): Promise<Holidays> => {
    if (op !== "listHolidays") throw new Error(`unexpected operation: ${op}`);
    await remote.answered;
    if (remote.failures > 0) {
      remote.failures--;
      throw { code: "external", message: "手机的连接已断开。", details: null } satisfies CallError;
    }
    return { daysOff: [["2026-10-01", "2026-10-03"]], makeUpDays: ["2026-10-10"] };
  },
}));

const unmounts: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const unmount of unmounts.splice(0)) await unmount();
  Object.assign(remote, { failures: 0, answered: Promise.resolve() });
});

async function mount() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <Calendar mode="single" defaultMonth={new Date(2026, 9, 1)} />
      </QueryClientProvider>,
    ),
  );
  unmounts.push(async () => {
    await act(async () => root.unmount());
    container.remove();
  });
  return {
    container,
    label: (day: string) => container.querySelector(`[data-day="${day}"] button`)?.getAttribute("aria-label"),
  };
}

it("marks the days off and the weekend days worked", async () => {
  const { label } = await mount();
  await vi.waitFor(() => expect(label("2026-10-01")).toMatch(/，休息日$/));
  expect(label("2026-10-03")).toMatch(/，休息日$/);
  expect(label("2026-10-10")).toMatch(/，调休上班$/);
  // The day after the holiday, an ordinary weekday and an ordinary weekend day.
  for (const day of ["2026-10-04", "2026-10-09", "2026-10-11"]) expect(label(day)).not.toMatch(/休息日|调休上班/);
});

it("says when the holidays could not be read, and reads them again", async () => {
  const gate: { open?: () => void } = {};
  remote.answered = new Promise((resolve) => (gate.open = resolve));
  remote.failures = 1;
  const { container, label } = await mount();
  // The region the words will go in is there before them, for a screen reader to announce them.
  const region = [...container.querySelectorAll('[role="status"]')].find((r) => r.textContent === "");
  expect(region).toBeDefined();
  await act(async () => gate.open?.());
  const note = "节假日标记未能载入：手机的连接已断开。";
  await vi.waitFor(() => expect(region?.textContent).toContain(note));
  // Meanwhile a day off reads as any other.
  expect(label("2026-10-01")).not.toMatch(/休息日/);
  const retry = [...container.querySelectorAll("button")].find((b) => b.textContent === "重试");
  await act(async () => retry?.click());
  await vi.waitFor(() => expect(label("2026-10-01")).toMatch(/，休息日$/));
  expect(container.textContent).not.toContain(note);
});
