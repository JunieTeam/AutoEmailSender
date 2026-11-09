// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { MentorRow } from "@/api/types";
import { MentorsPage } from "./mentors";

const remote = vi.hoisted((): { answer: (ids: number[]) => void } => ({ answer: () => undefined }));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <span>{children}</span>,
  useNavigate: () => () => undefined,
  useSearch: () => ({}),
}));
vi.mock("@/kept-scroll", () => ({ useKeptScroll: () => undefined }));
vi.mock("@/api/call", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/api/call")>()),
  call: async (op: string) => {
    if (op === "listMentors") return { mentors: [row(1), row(2)], total: 3 };
    if (op === "listMentorIds") return new Promise((resolve) => (remote.answer = resolve));
    if (op === "mentorFilterOptions") return { tracked: false };
    if (op === "matchingProgress") return { remaining: 0, failed: 0, pause: null };
    // The rest of the page, which this test does not look at.
    return [];
  },
}));

function row(id: number): MentorRow {
  return {
    id,
    name: `导师${id}`,
    affiliation: null,
    affiliationCount: 0,
    research: null,
    matchState: { result: null, state: null },
    contact: { status: "none" },
    outcome: null,
    tags: [],
  };
}

it("keeps the ticks cleared while every mentor was being ticked", async () => {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const button = (name: string) =>
    [...container.querySelectorAll<HTMLElement>("button, [role=checkbox]")].find(
      (b) => b.getAttribute("aria-label") === name || b.textContent === name,
    );
  const click = async (name: string) => {
    await vi.waitFor(() => expect(button(name)).toBeDefined());
    await act(async () => button(name)?.click());
  };
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <MentorsPage />
        </QueryClientProvider>,
      ),
    );
    await click("选择本页全部导师");
    await click("选择全部 3 位");
    await click("取消选择");
    await act(async () => remote.answer([1, 2, 3]));
    await vi.waitFor(() => expect(client.isMutating()).toBe(0));
    for (const name of ["选择本页全部导师", "选择导师1", "选择导师2"])
      expect(button(name)?.getAttribute("aria-checked")).toBe("false");
  } finally {
    await act(async () => root.unmount());
    client.clear();
    container.remove();
  }
});
