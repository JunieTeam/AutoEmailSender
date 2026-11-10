// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { ModelProvider, NewModelConfig } from "@/api/types";
import { ModelDialog } from "./model-dialog";

const remote = vi.hoisted(() => ({ added: vi.fn<(config: NewModelConfig) => void>() }));
vi.mock("@/api/call", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/api/call")>()),
  call: async (op: string, input: NewModelConfig) => {
    if (op === "addModelConfig") {
      remote.added(input);
      return { id: 1 };
    }
    throw new Error(`unexpected operation: ${op}`);
  },
}));

const NONE = { input: null, cache: null, output: null };

const PROVIDERS: ModelProvider[] = [
  {
    id: "deepseek",
    name: "DeepSeek",
    endpoints: [{ region: null, baseUrl: "https://api.deepseek.com", keyUrl: "https://platform.deepseek.com" }],
    currency: "cny",
    peakHours: {
      zone: "北京时间",
      weekdays: [
        [9, 12],
        [14, 18],
      ],
    },
    models: [
      {
        name: "deepseek-flash",
        prices: { input: 2, cache: 0.04, output: 8 },
        offPeak: { input: 1, cache: 0.02, output: 4 },
      },
    ],
  },
  {
    id: "qwen",
    name: "通义千问（阿里云百炼）",
    endpoints: [
      {
        region: { id: "cn-beijing", name: "北京" },
        baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        keyUrl: "https://bailian.console.aliyun.com",
      },
      {
        region: { id: "ap-southeast-1", name: "新加坡" },
        baseUrl: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
        keyUrl: "https://bailian.console.aliyun.com",
      },
    ],
    currency: "cny",
    peakHours: null,
    models: [{ name: "qwen3.7-plus", prices: { input: 2, cache: 0.4, output: 8 }, offPeak: null }],
  },
  {
    id: "openai",
    name: "OpenAI",
    endpoints: [{ region: null, baseUrl: "https://api.openai.com/v1", keyUrl: "https://platform.openai.com" }],
    currency: "usd",
    peakHours: null,
    models: [],
  },
];

function field(id: string) {
  const input = document.querySelector<HTMLInputElement>(`#${id}`);
  if (!input) throw new Error(`missing field: ${id}`);
  return input;
}

/** Types `text` in the field in place of what is in it and leaves it, as a user does. */
async function typeIn(id: string, text: string) {
  const input = field(id);
  await act(async () => {
    input.focus();
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, text);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => input.blur());
}

/** Opens the list of the field with that id and picks the option so named, as a click does. */
async function choose(id: string, option: string) {
  await act(async () => document.querySelector<HTMLElement>(`#${id}`)?.click());
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

/** What the group of price fields shows: input, cache hits, output. */
const prices = (group: string) => ["input", "cache", "output"].map((key) => field(`${group}-${key}`).value);

async function mount(run: () => Promise<void>) {
  const client = new QueryClient();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <ModelDialog config={null} providers={PROVIDERS} onClose={() => {}} />
        </QueryClientProvider>,
      ),
    );
    await run();
  } finally {
    await act(async () => root.unmount());
    client.clear();
    container.remove();
  }
}

it("fills in a known model's official prices and takes back only those it filled in", () =>
  mount(async () => {
    await choose("model-provider", "DeepSeek");
    await typeIn("model-model", "deepseek-flash");
    expect(prices("model-peak")).toEqual(["2", "0.04", "8"]);
    expect(prices("model-off-peak")).toEqual(["1", "0.02", "4"]);

    // What the user typed stays with another model, until a known one's prices replace it.
    await typeIn("model-peak-input", "3");
    await typeIn("model-model", "deepseek-chat");
    expect(prices("model-peak")).toEqual(["3", "0.04", "8"]);
    await typeIn("model-model", "deepseek-flash");
    expect(prices("model-peak")).toEqual(["2", "0.04", "8"]);
    // Prices filled in for a model are not those of one whose prices are not known.
    await typeIn("model-model", "deepseek-chat");
    expect(prices("model-peak")).toEqual(["", "", ""]);
    expect(prices("model-off-peak")).toEqual(["", "", ""]);

    await choose("model-provider", "OpenAI");
    expect(field("model-prices").textContent).toBe("单价（美元／百万 Token）");
    expect(document.querySelector("#model-off-peak-input")).toBeNull();
  }));

it("leaves out the prices in a region whose prices are not known, and saves the region", () =>
  mount(async () => {
    remote.added.mockReset();
    await choose("model-provider", "通义千问（阿里云百炼）");
    expect(document.body.textContent).toContain("接口地址：https://dashscope.aliyuncs.com/compatible-mode/v1");
    await typeIn("model-model", "qwen3.7-plus");
    expect(prices("model-price")).toEqual(["2", "0.4", "8"]);

    await choose("model-region", "新加坡");
    expect(document.body.textContent).toContain("接口地址：https://dashscope-intl.aliyuncs.com/compatible-mode/v1");
    expect(prices("model-price")).toEqual(["", "", ""]);
    await typeIn("model-api-key", "sk-test");
    const add = [...document.querySelectorAll("button")].find((b) => b.textContent === "添加");
    await act(async () => add?.click());
    expect(remote.added).toHaveBeenCalledWith(
      expect.objectContaining({
        provider: "qwen",
        region: "ap-southeast-1",
        model: "qwen3.7-plus",
        currency: "cny",
        prices: NONE,
        offPeak: NONE,
      }),
    );
  }));
