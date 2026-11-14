// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { ModelConfig, TaskSummary } from "@/api/types";
import { TaskReviewPage } from "./tasks";

const remote = vi.hoisted((): { task?: TaskSummary; calls: string[] } => ({ calls: [] }));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <span>{children}</span>,
  useParams: () => ({ id: 1 }),
  useSearch: () => ({}),
  useNavigate: () => async () => undefined,
  useBlocker: () => undefined,
  useRouterState: () => "",
  useRouter: () => ({}),
}));
vi.mock("@/api/call", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/api/call")>()),
  call: async (op: string) => {
    remote.calls.push(op);
    if (op === "getTask") return remote.task;
    if (op === "listTaskDrafts") return { drafts: [], total: 0 };
    if (op === "listModelConfigs") return models;
    if (op === "getAiFactCheck") return true;
    if (op.startsWith("get")) return null;
    // The rest of the page, which this test does not look at.
    return [];
  },
}));

// The task writes on the first; AI 核对 has since moved to the second, not agreed to yet.
const models: ModelConfig[] = [1, 2].map((id) => ({
  id,
  name: null,
  model: "fixture",
  provider: "custom",
  baseUrl: `http://provider${id}.example/v1`,
  region: null,
  currency: "cny",
  prices: { input: null, cache: null, output: null },
  offPeak: { input: null, cache: null, output: null },
  rate: null,
  agreed: id === 1,
  needsApiKey: false,
  isDefault: id === 1,
  features: id === 2 ? ["check"] : [],
}));

function paused(generating: number): TaskSummary {
  return {
    id: 1,
    name: "春季联系",
    writer: "ai",
    kind: "first",
    state: "paused",
    createdAt: 1,
    pauseReason: null,
    modelConfigId: 1,
    progress: { generating, review: 0, generationFailed: 0, approved: 1, cloud: 0, sent: 0, failed: 0, canceled: 0 },
    schedule: { kind: "now" },
  };
}

function button(name: string) {
  const node = Array.from(document.querySelectorAll("button")).find((b) => b.textContent === name);
  if (!node) throw new Error(`missing button: ${name}`);
  return node;
}

it("asks to agree to models on going on only while letters are left to write", async () => {
  remote.task = paused(0);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <TaskReviewPage />
        </QueryClientProvider>,
      ),
    );
    // Only sending is left, which calls no model.
    await vi.waitFor(() => expect(button("继续").disabled).toBe(false));
    await act(async () => button("继续").click());
    await vi.waitFor(() => expect(remote.calls).toContain("resumeTask"));
    expect(document.body.textContent).not.toContain("同意并继续");

    remote.calls = [];
    remote.task = paused(1);
    await act(async () => client.setQueryData(["getTask", 1], remote.task));
    await vi.waitFor(() => expect(button("继续").disabled).toBe(false));
    await act(async () => button("继续").click());
    await vi.waitFor(() => expect(document.body.textContent).toContain("provider2.example"));
    expect(remote.calls).not.toContain("resumeTask");
  } finally {
    await act(async () => root.unmount());
    client.clear();
    container.remove();
  }
});
