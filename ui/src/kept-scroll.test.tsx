// @vitest-environment happy-dom
import {
  RouterProvider,
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  useRouterState,
} from "@tanstack/react-router";
import { Activity, StrictMode, act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it } from "vitest";
import { useKeptScroll } from "./kept-scroll";

// The app registers its own routes, so these are reached by address rather than by typed route.
function List() {
  const search = new URLSearchParams(useRouterState({ select: (s) => s.location.searchStr }));
  // `stale`: the rows of the page before are kept on show while this one loads.
  const rows = { isSuccess: true, isPlaceholderData: search.has("stale") };
  const scroller = useKeptScroll<HTMLDivElement>(rows, search.get("page") ?? "1");
  return <div data-testid="list" ref={scroller} />;
}
// A tab kept while another is on show, as `Tabs` keeps them.
function Kept() {
  const hidden = new URLSearchParams(useRouterState({ select: (s) => s.location.searchStr })).has("other");
  return (
    <Activity mode={hidden ? "hidden" : "visible"}>
      <List />
    </Activity>
  );
}
const root = createRootRoute();
const routeTree = root.addChildren([
  createRoute({ getParentRoute: () => root, path: "/", component: List }),
  createRoute({ getParentRoute: () => root, path: "/mentor", component: () => <p>导师</p> }),
  createRoute({ getParentRoute: () => root, path: "/tabs", component: Kept }),
]);

let unmount = () => {};
afterEach(() => unmount());

async function app() {
  const router = createRouter({ routeTree, history: createMemoryHistory({ initialEntries: ["/"] }) });
  const container = document.createElement("div");
  document.body.append(container);
  const view = createRoot(container);
  // As the app renders, effects running twice on mounting included.
  await act(async () =>
    view.render(
      <StrictMode>
        <RouterProvider router={router} />
      </StrictMode>,
    ),
  );
  unmount = () => {
    act(() => view.unmount());
    container.remove();
  };
  const go = async (step: (history: typeof router.history) => void) => {
    await act(async () => {
      step(router.history);
      await router.load();
    });
  };
  const list = () => container.querySelector<HTMLElement>('[data-testid="list"]');
  return { go, list };
}

it("returns to where a list was when going back, and starts another page at the top", async () => {
  const { go, list } = await app();
  list()?.scrollTo({ top: 600 });

  await go((h) => h.push("/mentor"));
  expect(list()).toBeNull();
  await go((h) => h.back());
  expect(list()?.scrollTop).toBe(600);

  await go((h) => h.push("/?page=2"));
  expect(list()?.scrollTop).toBe(0);
  await go((h) => h.back());
  expect(list()?.scrollTop).toBe(600);
});

it("leaves a list where it is when only something beside it opens", async () => {
  const { go, list } = await app();
  list()?.scrollTo({ top: 400 });
  await go((h) => h.push("/?open=7"));
  expect(list()?.scrollTop).toBe(400);
});

it("waits for the rows of the new page before moving the list", async () => {
  const { go, list } = await app();
  list()?.scrollTo({ top: 500 });
  await go((h) => h.push("/?page=2&stale"));
  expect(list()?.scrollTop).toBe(500);
  await go((h) => h.replace("/?page=2"));
  expect(list()?.scrollTop).toBe(0);
});

it("returns to where a list was left when the sidebar goes back to its page", async () => {
  const { go, list } = await app();
  list()?.scrollTo({ top: 300 });
  await go((h) => h.push("/mentor"));
  // A step of its own, not back in the history.
  await go((h) => h.push("/", { returning: true }));
  expect(list()?.scrollTop).toBe(300);
  await go((h) => h.push("/mentor"));
  await go((h) => h.push("/"));
  expect(list()?.scrollTop).toBe(0);
});

it("puts back where a list in a kept tab was, which the browser forgets while it is hidden", async () => {
  const { go, list } = await app();
  await go((h) => h.push("/tabs"));
  list()?.scrollTo({ top: 450 });
  await go((h) => h.push("/tabs?other"));
  // As a browser does with what display: none hides.
  list()?.scrollTo({ top: 0 });
  await go((h) => h.push("/tabs"));
  expect(list()?.scrollTop).toBe(450);
});
