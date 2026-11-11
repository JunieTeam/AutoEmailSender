// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { Holidays } from "@/api/types";
import { Calendar } from "./calendar";

vi.mock("@/api/call", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/api/call")>()),
  call: async (op: string): Promise<Holidays> => {
    if (op === "listHolidays") return { daysOff: [["2026-10-01", "2026-10-03"]], makeUpDays: ["2026-10-10"] };
    throw new Error(`unexpected operation: ${op}`);
  },
}));

it("marks the days off and the weekend days worked", async () => {
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
  const label = (day: string) => container.querySelector(`[data-day="${day}"] button`)?.getAttribute("aria-label");
  await vi.waitFor(() => expect(label("2026-10-01")).toMatch(/，休息日$/));
  expect(label("2026-10-03")).toMatch(/，休息日$/);
  expect(label("2026-10-10")).toMatch(/，调休上班$/);
  // The day after the holiday, an ordinary weekday and an ordinary weekend day.
  for (const day of ["2026-10-04", "2026-10-09", "2026-10-11"]) expect(label(day)).not.toMatch(/休息日|调休上班/);
  await act(async () => root.unmount());
  container.remove();
});
