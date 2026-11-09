// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  RouterProvider,
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
} from "@tanstack/react-router";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { CandidateRow } from "@/api/types";
import { FindPage, findSearch } from "./find";

vi.mock("@/api/call", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/api/call")>()),
  call: async (op: string, input: { keyword?: string }) => {
    if (op === "listCandidates") return { candidates: [candidate(input.keyword ?? "")], total: 1 };
    // The rest of the page, which this test does not look at.
    return [];
  },
}));

function candidate(keyword: string): CandidateRow {
  return {
    id: keyword.length,
    crawlId: null,
    name: `候选${keyword}`,
    emails: [],
    affiliation: null,
    research: null,
    sourceUrl: null,
    review: "pending",
    mentorId: null,
    category: "new",
    issue: null,
    enrich: "none",
    enrichChanges: [],
    enrichError: null,
  };
}

// The app registers its own routes, so this one is reached by address rather than by typed route.
const root = createRootRoute();
const routeTree = root.addChildren([
  createRoute({ getParentRoute: () => root, path: "/find", validateSearch: findSearch, component: FindPage }),
]);

it("drops the ticks of another search when going back to the one before", async () => {
  const router = createRouter({ routeTree, history: createMemoryHistory({ initialEntries: ["/find?keyword=A"] }) });
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const container = document.createElement("div");
  document.body.append(container);
  const view = createRoot(container);
  const tick = (name: string) => container.querySelector<HTMLElement>(`[aria-label="选择${name}"]`);
  // While sinking away the bar still reads as it did, so what counts is whether it is on show.
  const bar = () => [...container.querySelectorAll("span")].find((s) => s.textContent?.startsWith("已选"));
  try {
    await act(async () =>
      view.render(
        <QueryClientProvider client={client}>
          <RouterProvider router={router} />
        </QueryClientProvider>,
      ),
    );
    await vi.waitFor(() => expect(tick("候选A")).not.toBeNull());
    await act(async () => router.navigate({ to: "/find", search: findSearch({ keyword: "BB" }) }));
    await vi.waitFor(() => expect(tick("候选BB")).not.toBeNull());
    // As a user does, on the row's label around the box.
    await act(async () => tick("候选BB")?.closest("label")?.click());
    await vi.waitFor(() => expect(bar()?.closest("[inert]")).toBeNull());
    await act(async () => router.history.back());
    await vi.waitFor(() => expect(tick("候选A")).not.toBeNull());
    expect(bar()?.closest("[inert]")).not.toBeNull();
  } finally {
    await act(async () => view.unmount());
    client.clear();
    container.remove();
  }
});
