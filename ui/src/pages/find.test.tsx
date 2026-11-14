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
import { beforeEach, expect, it, vi } from "vitest";
import type { CandidateDetail, CandidateRow, CrawlSummary, IntakePreview, ModelConfig } from "@/api/types";
import { FindPage, findSearch, ResumeCrawl } from "./find";

// What the candidates' list shows: by default one candidate named after the keyword.
const remote = vi.hoisted(
  (): { list: (keyword: string) => CandidateRow[]; total: number; ids: number[]; rejected: number[][] } => ({
    list: (keyword) => [candidate(keyword.length, `候选${keyword}`)],
    total: 1,
    ids: [],
    rejected: [],
  }),
);
vi.mock("@/api/call", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/api/call")>()),
  call: async (op: string, input: { keyword?: string; id?: number; ids?: number[] }) => {
    if (op === "listCandidates") {
      const candidates = remote.list(input.keyword ?? "");
      const n = candidates.length;
      return { candidates, total: remote.total, counts: { all: n, new: n, supplement: 0, maybe: 0, missingEmail: n } };
    }
    if (op === "listCandidateIds") return remote.ids;
    if (op === "getCandidate") return detail(input.id ?? 0);
    if (op === "previewCandidates") return preview;
    if (op === "rejectCandidates") {
      remote.rejected.push(input.ids ?? []);
      return input.ids?.length;
    }
    if (op === "listModelConfigs") return [model];
    // The rest of the page, which these tests do not look at.
    return [];
  },
}));

beforeEach(() => {
  remote.list = (keyword) => [candidate(keyword.length, `候选${keyword}`)];
  remote.total = 1;
  remote.ids = [];
  remote.rejected = [];
});

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

function candidate(id: number, name: string): CandidateRow {
  return {
    id,
    crawlId: null,
    name,
    emails: [],
    affiliation: null,
    research: null,
    sourceUrl: null,
    review: "pending",
    mentorId: null,
    category: "new",
    issue: null,
    recognized: [],
    enrich: "none",
    enrichChanges: [],
    enrichError: null,
  };
}

const NAMES: Record<number, string> = { 1: "候选甲", 2: "候选乙" };
function detail(id: number): CandidateDetail {
  return {
    id,
    crawlId: null,
    review: "pending",
    mentorId: null,
    fields: {
      name: NAMES[id] ?? `候选${id}`,
      aliases: [],
      emails: [],
      affiliations: [],
      homepages: [],
      research: null,
    },
    papers: [],
    sourceUrl: null,
    note: null,
    enrich: "none",
    enrichChanges: [],
    enrichError: null,
    evidence: [],
    recognized: [],
  };
}

const preview: IntakePreview = {
  created: 1,
  supplemented: 0,
  updated: 0,
  unchanged: 0,
  differences: [],
  additions: [],
  maybeSame: [],
  trashed: [],
  errors: [],
};

// The app registers its own routes, so this one is reached by address rather than by typed route.
const root = createRootRoute();
const routeTree = root.addChildren([
  createRoute({ getParentRoute: () => root, path: "/find", validateSearch: findSearch, component: FindPage }),
]);

function routerAt(address: string) {
  return createRouter({ routeTree, history: createMemoryHistory({ initialEntries: [address] }) });
}

/** Shows 找导师 at `address`, gives it to `test`, and clears it all away after. */
async function page(
  address: string,
  test: (container: HTMLElement, router: ReturnType<typeof routerAt>) => Promise<void>,
) {
  const router = routerAt(address);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const container = document.createElement("div");
  document.body.append(container);
  const view = createRoot(container);
  try {
    await act(async () =>
      view.render(
        <QueryClientProvider client={client}>
          <RouterProvider router={router} />
        </QueryClientProvider>,
      ),
    );
    await test(container, router);
  } finally {
    await act(async () => view.unmount());
    client.clear();
    container.remove();
  }
}

// While sinking away the bar still reads as it did, so what counts is whether it is on show.
const bar = (container: HTMLElement) =>
  [...container.querySelectorAll("span")].find((s) => s.textContent?.startsWith("已选"));

it("drops the ticks of another search when going back to the one before", async () => {
  await page("/find?keyword=A", async (container, router) => {
    const tick = (name: string) => container.querySelector<HTMLElement>(`[aria-label="选择${name}"]`);
    await vi.waitFor(() => expect(tick("候选A")).not.toBeNull());
    await act(async () => router.navigate({ to: "/find", search: findSearch({ keyword: "BB" }) }));
    await vi.waitFor(() => expect(tick("候选BB")).not.toBeNull());
    // As a user does, on the row's label around the box.
    await act(async () => tick("候选BB")?.closest("label")?.click());
    await vi.waitFor(() => expect(bar(container)?.closest("[inert]")).toBeNull());
    await act(async () => router.history.back());
    await vi.waitFor(() => expect(tick("候选A")).not.toBeNull());
    expect(bar(container)?.closest("[inert]")).not.toBeNull();
  });
});

it("ticks every candidate the filter finds, on the pages not shown too", async () => {
  remote.list = () => [candidate(1, "候选甲")];
  remote.total = 3;
  remote.ids = [1, 2, 3];
  await page("/find", async (container) => {
    const all = () => [...container.querySelectorAll("label")].find((l) => l.textContent === "全选筛出的 3 位");
    await vi.waitFor(() => expect(all()).toBeDefined());
    await act(async () => all()?.click());
    await vi.waitFor(() => expect(bar(container)?.textContent).toBe("已选 3 位"));
    expect(bar(container)?.closest("[inert]")).toBeNull();
  });
});

it("goes on to the next candidate once the one on show is rejected", async () => {
  remote.list = () =>
    [candidate(1, "候选甲"), candidate(2, "候选乙")].filter((c) => !remote.rejected.flat().includes(c.id));
  remote.total = 2;
  await page("/find", async (container) => {
    const name = () => container.querySelector("h2")?.textContent;
    const reject = () => [...container.querySelectorAll("button")].find((b) => b.textContent === "拒绝");
    await vi.waitFor(() => expect(name()).toBe("候选甲"));
    await vi.waitFor(() => expect(reject()?.disabled).toBe(false));
    await act(async () => reject()?.click());
    await vi.waitFor(() => expect(name()).toBe("候选乙"));
    expect(remote.rejected).toEqual([[1]]);
  });
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
    profilesDone: 0,
    candidates: 1,
    withEmail: 0,
    pending: 1,
    failed: 1,
    retryable: 1,
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
