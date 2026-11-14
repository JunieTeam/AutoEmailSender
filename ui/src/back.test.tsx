// @vitest-environment happy-dom
import {
  RouterProvider,
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
} from "@tanstack/react-router";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it } from "vitest";
import { openSection } from "./back";

const root = createRootRoute();
const routeTree = root.addChildren([
  createRoute({ getParentRoute: () => root, path: "/mentors", component: () => <p>导师</p> }),
  createRoute({ getParentRoute: () => root, path: "/mentors/$id", component: () => <p>导师详情</p> }),
  createRoute({ getParentRoute: () => root, path: "/tasks", component: () => <p>任务</p> }),
]);

let unmount = () => {};
afterEach(() => unmount());

it("opens a section in place of the pages opened before, so that going back stops there", async () => {
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
  expect(router.history.canGoBack()).toBe(true);

  await act(() => openSection(router, "/tasks"));
  expect(container.textContent).toBe("任务");
  expect(router.history.canGoBack()).toBe(false);
});
