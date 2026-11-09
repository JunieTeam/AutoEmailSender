// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { LetterStyle } from "@/api/types";
import { LetterStyleSection } from "./letter-style";

const remote = vi.hoisted(() => {
  const style: LetterStyle = {
    font: null,
    fontSizePt: 12,
    lineHeight: 1.6,
    paragraphSpacingEm: 0.4,
    firstLineIndent: false,
  };
  return { style, save: vi.fn<(style: LetterStyle) => Promise<null>>() };
});
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <span>{children}</span>,
}));
vi.mock("@/api/call", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/api/call")>()),
  call: (op: string, input: LetterStyle) => {
    if (op === "updateLetterStyle") return remote.save(input);
    if (op === "getLetterStyle") return structuredClone(remote.style);
    throw new Error(`unexpected operation: ${op}`);
  },
  errorMessage: (error: Error) => error.message,
}));

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

it("shows what an Agent saved and does not save the old style back", async () => {
  remote.save.mockReset().mockImplementation(async (style) => {
    remote.style = style;
    return null;
  });
  const client = new QueryClient({ defaultOptions: { queries: { staleTime: Infinity } } });
  client.setQueryData(["getLetterStyle"], remote.style);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <LetterStyleSection />
        </QueryClientProvider>,
      ),
    );
    // An Agent saves while the page is open.
    remote.style = { ...remote.style, fontSizePt: 14 };
    await act(async () => client.setQueryData(["getLetterStyle"], remote.style));
    await vi.waitFor(() => expect(document.querySelector("#letter-size")?.textContent).toBe("四号（14 磅）"));
    await choose("letter-spacing", "中");
    expect(remote.save).toHaveBeenLastCalledWith(expect.objectContaining({ fontSizePt: 14, paragraphSpacingEm: 0.8 }));
  } finally {
    await act(async () => root.unmount());
    client.clear();
    container.remove();
  }
});
