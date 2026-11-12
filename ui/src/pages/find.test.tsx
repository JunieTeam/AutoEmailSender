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
import type { CandidateRow, CrawlSummary, ModelConfig } from "@/api/types";
import { FindPage, findSearch, ResumeCrawl } from "./find";

vi.mock("@/api/call", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/api/call")>()),
  call: async (op: string, input: { keyword?: string }) => {
    if (op === "listCandidates") return { candidates: [candidate(input.keyword ?? "")], total: 1 };
    if (op === "listModelConfigs") return [model];
    // The rest of the page, which this test does not look at.
    return [];
  },
}));

// The only model, which 智能整理 follows as the default, not agreed to yet.
const model: ModelConfig = {
  id: 1,
  name: null,
  model: "fixture",
  provider: "custom",
  baseUrl: "http://provider.example/v1",
  region: null,
  currency: "cny",
  prices: { input: null, cache: null, output: null },
  offPeak: { input: null, cache: null, output: null },
  rate: null,
  agreed: false,
  needsApiKey: false,
  isDefault: true,
  features: [],
};

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

function paused(remaining: number): CrawlSummary {
  return {
    id: 3,
    kind: "site",
    url: "https://school.example.edu/",
    university: null,
    school: null,
    modelConfigId: 1,
    state: "paused",
    error: null,
    createdAt: 1,
    finishedAt: null,
    pages: 2,
    remaining,
    enrichRemaining: 0,
    modelIds: remaining > 0 ? [1] : [],
    retryModelIds: [1],
    profiles: 0,
    candidates: 1,
    failed: 1,
    tokens: null,
    stage: "paused",
  };
}

it("goes on with a paused crawl at once when no page is left to read", async () => {
  const resumed: number[] = [];
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const container = document.createElement("div");
  document.body.append(container);
  const view = createRoot(container);
  const render = (remaining: number) =>
    act(async () =>
      view.render(
        <QueryClientProvider client={client}>
          <ResumeCrawl crawl={paused(remaining)} disabled={false} onResume={() => resumed.push(remaining)} />
        </QueryClientProvider>,
      ),
    );
  const button = () => [...container.querySelectorAll("button")].find((b) => b.textContent === "继续");
  try {
    // Rejecting the candidates whose pages were left to read leaves nothing for a model to read.
    await render(0);
    await act(async () => button()?.click());
    expect(resumed).toEqual([0]);
    await render(1);
    await vi.waitFor(() => expect(button()?.disabled).toBe(false));
    await act(async () => button()?.click());
    await vi.waitFor(() => expect(document.body.textContent).toContain("同意并继续"));
    expect(resumed).toEqual([0]);
  } finally {
    await act(async () => view.unmount());
    client.clear();
    container.remove();
  }
});
