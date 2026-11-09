// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { MentorFilters } from "./mentor-filters";

vi.mock("@/api/call", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/api/call")>()),
  call: async (op: string) => {
    if (op === "listTags") throw new Error("tags unavailable");
    if (op === "mentorFilterTags") return { tags: [{ id: 1, name: "待联系", count: 4 }], untagged: 0 };
    return { values: [], missing: 0 };
  },
}));

it("names the tags in the menu as the filter lists them, whatever else failed", async () => {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <MentorFilters search={{}} filter={{}} onChange={() => undefined} />
        </QueryClientProvider>,
      ),
    );
    const trigger = [...container.querySelectorAll("button")].find((b) => b.textContent === "标签");
    await act(async () => trigger?.click());
    await vi.waitFor(() => expect(document.body.textContent).toContain("待联系"));
  } finally {
    await act(async () => root.unmount());
    client.clear();
    container.remove();
  }
});
