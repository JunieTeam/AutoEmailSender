// @vitest-environment happy-dom
import {
  Outlet,
  RouterProvider,
  createHashHistory,
  createRootRoute,
  createRoute,
  createRouter,
  useRouter,
  useRouterState,
} from "@tanstack/react-router";
import { StrictMode, act, useState } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it } from "vitest";
import { formClose } from "@/components/form";
import { Dialog, DialogContent, DialogHeader } from "@/components/ui/dialog";
import { LayerSteps, openSection, useBeforeLeave, useLayerStep } from "./back";

declare global {
  interface Window {
    happyDOM: { waitUntilComplete: () => Promise<void> };
  }
}

/** While set, leaving the page waits for it, as the task review page waits for a save. */
let saving: Promise<void> | undefined;
const waitForSave = async () => {
  await saving;
  return false;
};

/**
 * A page with a dialog, which `改动` gives unsaved input and which opens another on top; a dialog
 * unsaved from the start; one the address opens; and a panel to choose a school in, which
 * `两个都打开` opens together with the first dialog.
 */
function Page() {
  const router = useRouter();
  const [open, setOpen] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [noting, setNoting] = useState(false);
  const [noted, setNoted] = useState(false);
  const [timing, setTiming] = useState(false);
  const detail = useRouterState({ select: (s) => "detail" in s.location.search });
  const [picking, setPicking] = useState(false);
  useLayerStep(picking, () => setPicking(false));
  useBeforeLeave(waitForSave, () => false);
  return (
    <>
      <p>导师详情</p>
      <button onClick={() => setOpen(true)}>打开弹窗</button>
      <button onClick={() => setTiming(true)}>改时间</button>
      <button onClick={() => void router.navigate({ href: "/mentors/1?detail=1", replace: true })}>看记录</button>
      <button onClick={() => setPicking(true)}>选学校</button>
      <button
        onClick={() => {
          setPicking(true);
          setOpen(true);
        }}
      >
        两个都打开
      </button>
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
            <button onClick={() => setNoting(true)}>加备注</button>
            <button
              onClick={() => {
                setOpen(false);
                void router.navigate({ href: "/tasks" });
              }}
            >
              去任务
            </button>
            {noting && (
              <Dialog open {...formClose(noted, false, () => setNoting(false))}>
                <DialogContent>
                  <DialogHeader>备注</DialogHeader>
                  <button onClick={() => setNoted(true)}>写备注</button>
                </DialogContent>
              </Dialog>
            )}
          </DialogContent>
        </Dialog>
      )}
      {timing && (
        // As the dialog that changes a sending time: what it shows is not saved until confirmed.
        <Dialog open {...formClose(true, false, () => setTiming(false))}>
          <DialogContent>
            <DialogHeader>改发送时间</DialogHeader>
          </DialogContent>
        </Dialog>
      )}
      {detail && (
        // As a crawl's details: closing takes them out of the page's address.
        <Dialog open onOpenChange={(o) => !o && void router.navigate({ href: "/mentors/1", replace: true })}>
          <DialogContent>
            <DialogHeader>整理记录</DialogHeader>
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
const questions = () => [...document.querySelectorAll('[role="alert"]')];

// A browser goes back and forward in a later task, which is when the app's own navigations, held for
// the router's checks, can come first; happy-dom does it at once.
const { history } = window;
const at = { go: history.go.bind(history), back: history.back.bind(history), forward: history.forward.bind(history) };
/** Runs as going back or forward is asked for, before the browser does it. */
let asked: (() => void) | undefined;
history.go = (delta) => {
  asked?.();
  setTimeout(() => at.go(delta));
};
history.back = () => {
  asked?.();
  setTimeout(at.back);
};
history.forward = () => {
  asked?.();
  setTimeout(at.forward);
};

/** Lets the browser and the app finish what follows from `act`, as going back or forward does. */
async function settle(act_: () => void) {
  await act(async () => {
    act_();
    await window.happyDOM.waitUntilComplete();
  });
  await act(() => window.happyDOM.waitUntilComplete());
}

let unmount = async () => {};
afterEach(() => unmount());

/** The app in the browser on a mentor's page, opened from the list of mentors. */
async function render() {
  // The router numbers its entries from the one it starts on; going back past it shows another site.
  window.history.replaceState(null, "", "#/elsewhere");
  window.history.pushState(null, "", "#/mentors");
  const router = createRouter({ routeTree, history: createHashHistory() });
  const container = document.createElement("div");
  document.body.append(container);
  const view = createRoot(container);
  // As in main.tsx, which runs each effect twice as it mounts.
  await act(async () =>
    view.render(
      <StrictMode>
        <RouterProvider router={router} />
      </StrictMode>,
    ),
  );
  unmount = async () => {
    // Nothing left over to go back or forward once the next test has begun.
    await act(() => window.happyDOM.waitUntilComplete());
    act(() => view.unmount());
    router.history.destroy();
    container.remove();
    saving = undefined;
    asked = undefined;
  };
  await settle(() => void router.navigate({ href: "/mentors/1" }));
  return {
    router,
    button,
    click: (name: string) => settle(() => button(name)?.click()),
    // As the phone's swipe or a long press on the browser's back button does.
    back: (steps = 1) => settle(() => window.history.go(-steps)),
    forward: () => settle(() => window.history.forward()),
    shows: (text: string) => document.body.textContent?.includes(text) ?? false,
    page: () => container.textContent,
  };
}

it("opens a section in place of the pages opened before, so that going back stops there", async () => {
  const app = await render();
  expect(app.router.history.canGoBack()).toBe(true);

  await settle(() => void openSection(app.router, "/tasks"));
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

it("asks before leaving the page with unsaved input, going back several steps at once", async () => {
  const app = await render();
  await app.click("打开弹窗");
  await app.click("改动");

  await app.back(2);
  expect(app.shows("放弃未保存的内容？")).toBe(true);
  await app.click("继续编辑");
  expect(app.shows("编辑导师")).toBe(true);
  expect(app.page()).toContain("导师详情");

  await app.back(2);
  await app.click("放弃");
  expect(app.page()).toBe("导师");
});

it("takes the step of a dialog closed by its own button back with it", async () => {
  const app = await render();
  await app.click("打开弹窗");
  await settle(() => document.querySelector<HTMLButtonElement>('button[aria-label="关闭"]')?.click());
  expect(app.shows("编辑导师")).toBe(false);

  // Forward does not open it again, nor leave an empty step to go back over.
  await app.forward();
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

it("passes over the step of a dialog that opened another page, both ways", async () => {
  const app = await render();
  await app.click("打开弹窗");
  await app.click("去任务");
  expect(app.page()).toBe("任务");

  await app.back();
  expect(app.page()).toContain("导师详情");
  expect(app.shows("编辑导师")).toBe(false);
  await app.forward();
  expect(app.page()).toBe("任务");
  await app.back();
  await app.back();
  expect(app.page()).toBe("导师");
});

it("gives each of the layers opened together a step of its own", async () => {
  const app = await render();
  const open = () => [app.shows("编辑导师"), app.button("示例大学") !== undefined].filter(Boolean).length;
  await app.click("两个都打开");
  expect(open()).toBe(2);

  await app.back();
  expect(open()).toBe(1);
  await app.back();
  expect(open()).toBe(0);
  expect(app.page()).toContain("导师详情");
  await app.back();
  expect(app.page()).toBe("导师");
});

it("goes back no further for a dialog closed while going back waits for a save", async () => {
  const app = await render();
  await app.click("打开弹窗");
  let saved: (() => void) | undefined;
  saving = new Promise((resolve) => (saved = resolve));

  await app.back();
  await settle(() => document.querySelector<HTMLButtonElement>('button[aria-label="关闭"]')?.click());
  await settle(() => saved?.());
  expect(app.page()).toContain("导师详情");
  expect(app.shows("编辑导师")).toBe(false);
  expect(app.router.history.canGoBack()).toBe(true);
  saving = undefined;
  await app.back();
  expect(app.page()).toBe("导师");
});

it("goes on to the page a closing dialog opens while the page in front saves first", async () => {
  const app = await render();
  await app.click("打开弹窗");
  let saved: (() => void) | undefined;
  saving = new Promise((resolve) => (saved = resolve));
  // The save may end after the browser is asked to go back over the dialog's step, before it does.
  asked = () => saved?.();

  await app.click("去任务");
  await settle(() => saved?.());
  expect(app.page()).toBe("任务");
  saving = undefined;
  await app.back();
  expect(app.page()).toContain("导师详情");
  expect(app.shows("编辑导师")).toBe(false);
  await app.back();
  expect(app.page()).toBe("导师");
});

it("closes a dialog the address opens on going back, and goes back further past it", async () => {
  const app = await render();
  await app.click("看记录");
  expect(app.shows("整理记录")).toBe(true);

  await app.back();
  expect(app.shows("整理记录")).toBe(false);
  expect(app.router.state.location.href).toBe("/mentors/1");
  await app.click("看记录");
  await app.back(2);
  expect(app.page()).toBe("导师");
});

it("asks once, on top, before throwing away the input of dialogs one over another", async () => {
  const app = await render();
  await app.click("打开弹窗");
  await app.click("改动");
  await app.click("加备注");
  await app.click("写备注");

  await app.back(2);
  expect(questions()).toHaveLength(1);
  expect(questions()[0]?.closest("[data-nested-dialog-open]")).toBeNull();
  await app.click("放弃");
  expect(questions()).toHaveLength(0);
  expect(app.shows("备注")).toBe(false);
  expect(app.shows("编辑导师")).toBe(false);
  expect(app.page()).toContain("导师详情");
});

it("takes back the question once the browser goes forward again", async () => {
  const app = await render();
  await app.click("打开弹窗");
  await app.click("改动");
  await app.back();
  expect(app.shows("放弃未保存的内容？")).toBe(true);

  await app.forward();
  expect(app.shows("放弃未保存的内容？")).toBe(false);
  expect(app.shows("编辑导师")).toBe(true);
  await app.back();
  await app.click("放弃");
  expect(app.shows("编辑导师")).toBe(false);
  expect(app.page()).toContain("导师详情");
});

it("takes back the question once the app goes on to another address of the page", async () => {
  const app = await render();
  await app.click("打开弹窗");
  await app.click("改动");
  await app.back();
  expect(app.shows("放弃未保存的内容？")).toBe(true);

  await settle(() => void app.router.navigate({ href: "/mentors/1?page=2" }));
  expect(app.shows("放弃未保存的内容？")).toBe(false);
  expect(app.shows("编辑导师")).toBe(true);
  await app.back();
  await app.click("放弃");
  expect(app.shows("编辑导师")).toBe(false);
  expect(app.page()).toContain("导师详情");
});

it("asks about a dialog's input before its step is in the history", async () => {
  const app = await render();
  // The browser goes back as the dialog opens, before its step is added.
  await settle(() => {
    flushSync(() => app.button("改时间")?.click());
    at.back();
  });
  expect(app.shows("放弃未保存的内容？")).toBe(true);
  await app.click("继续编辑");
  expect(app.shows("改发送时间")).toBe(true);
  expect(app.page()).toContain("导师详情");
});
