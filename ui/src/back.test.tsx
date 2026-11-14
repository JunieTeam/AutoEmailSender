// @vitest-environment happy-dom
import {
  Outlet,
  RouterProvider,
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  useRouter,
} from "@tanstack/react-router";
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it } from "vitest";
import { formClose } from "@/components/form";
import { Dialog, DialogContent, DialogHeader } from "@/components/ui/dialog";
import { LayerSteps, openSection, useLayerStep } from "./back";

/** A page with a dialog, which `改动` gives unsaved input, and a panel to choose a school in. */
function Page() {
  const router = useRouter();
  const [open, setOpen] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [picking, setPicking] = useState(false);
  useLayerStep(picking, () => setPicking(false));
  return (
    <>
      <p>导师详情</p>
      <button onClick={() => setOpen(true)}>打开弹窗</button>
      <button onClick={() => setPicking(true)}>选学校</button>
      {picking && (
        <button
          onClick={() => {
            setPicking(false);
            void router.navigate({ href: "/mentors/1?school=1", replace: true });
          }}
        >
          示例大学
        </button>
      )}
      {open && (
        <Dialog open {...formClose(dirty, false, () => setOpen(false))}>
          <DialogContent>
            <DialogHeader>编辑导师</DialogHeader>
            <button onClick={() => setDirty(true)}>改动</button>
            <button
              onClick={() => {
                setOpen(false);
                void router.navigate({ href: "/tasks" });
              }}
            >
              去任务
            </button>
          </DialogContent>
        </Dialog>
      )}
    </>
  );
}

const root = createRootRoute({
  component: () => (
    <>
      <Outlet />
      <LayerSteps />
    </>
  ),
});
const routeTree = root.addChildren([
  createRoute({ getParentRoute: () => root, path: "/mentors", component: () => <p>导师</p> }),
  createRoute({ getParentRoute: () => root, path: "/mentors/$id", component: Page }),
  createRoute({ getParentRoute: () => root, path: "/tasks", component: () => <p>任务</p> }),
]);

const button = (name: string) => [...document.querySelectorAll("button")].find((b) => b.textContent === name);

let unmount = () => {};
afterEach(() => unmount());

/** The app on a mentor's page, opened from the list of mentors. */
async function render() {
  const router = createRouter({ routeTree, history: createMemoryHistory({ initialEntries: ["/mentors"] }) });
  const container = document.createElement("div");
  document.body.append(container);
  const view = createRoot(container);
  await act(async () => view.render(<RouterProvider router={router} />));
  unmount = () => {
    act(() => view.unmount());
    container.remove();
  };
  await act(() => router.navigate({ href: "/mentors/1" }));
  return {
    router,
    button,
    click: (name: string) => act(async () => button(name)?.click()),
    back: () => act(async () => router.history.back()),
    shows: (text: string) => document.body.textContent?.includes(text) ?? false,
    page: () => container.textContent,
  };
}

it("opens a section in place of the pages opened before, so that going back stops there", async () => {
  const app = await render();
  expect(app.router.history.canGoBack()).toBe(true);

  await act(() => openSection(app.router, "/tasks"));
  expect(app.page()).toBe("任务");
  expect(app.router.history.canGoBack()).toBe(false);
});

it("closes the open dialog on going back, and stays on the page", async () => {
  const app = await render();
  await app.click("打开弹窗");
  expect(app.shows("编辑导师")).toBe(true);

  await app.back();
  expect(app.shows("编辑导师")).toBe(false);
  expect(app.page()).toContain("导师详情");
  await app.back();
  expect(app.page()).toBe("导师");
});

it("asks before throwing away unsaved input on going back", async () => {
  const app = await render();
  await app.click("打开弹窗");
  await app.click("改动");

  await app.back();
  expect(app.shows("放弃未保存的内容？")).toBe(true);
  await app.click("继续编辑");
  expect(app.shows("放弃未保存的内容？")).toBe(false);
  expect(app.shows("编辑导师")).toBe(true);

  // Still a step: going back asks again.
  await app.back();
  await app.click("放弃");
  expect(app.shows("编辑导师")).toBe(false);
  expect(app.page()).toContain("导师详情");
  await app.back();
  expect(app.page()).toBe("导师");
});

it("takes the step of a dialog closed by its own button back with it", async () => {
  const app = await render();
  await app.click("打开弹窗");
  await act(async () => document.querySelector<HTMLButtonElement>('button[aria-label="关闭"]')?.click());
  expect(app.shows("编辑导师")).toBe(false);

  await app.back();
  expect(app.page()).toBe("导师");
});

it("keeps what was chosen in a panel with the page it closes back to", async () => {
  const app = await render();
  await app.click("选学校");
  await app.click("示例大学");
  expect(app.router.state.location.href).toBe("/mentors/1?school=1");
  expect(app.button("示例大学")).toBeUndefined();

  await app.back();
  expect(app.page()).toBe("导师");
});

it("passes over the step of a dialog that opened another page", async () => {
  const app = await render();
  await app.click("打开弹窗");
  await app.click("去任务");
  expect(app.page()).toBe("任务");

  await app.back();
  expect(app.page()).toContain("导师详情");
  expect(app.shows("编辑导师")).toBe(false);
  await app.back();
  expect(app.page()).toBe("导师");
});
