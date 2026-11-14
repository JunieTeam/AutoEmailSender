import { type AnyRouter, useRouter, useRouterState } from "@tanstack/react-router";
import { ChevronLeft } from "lucide-react";
import { useEffect, useEffectEvent, useReducer, useRef } from "react";
import { Button } from "@/components/ui/button";

// docs/ui.md#交互: the history keeps only what was opened, so that going back, by an arrow or the
// phone's swipe, returns to where it was opened from.

/**
 * Opens a section of the sidebar or the tab bar in place of all opened before it, so that going
 * back stops there: on a phone, swiping back on a section leads to no other. The page in front may
 * hold on first, as it may for any navigation, and then stays.
 */
export function openSection(router: AnyRouter, href: string, returning = false): Promise<void> {
  const open = (ignoreBlocker: boolean) =>
    router.navigate({ href, replace: true, state: { returning }, ignoreBlocker });
  const { history } = router;
  const depth = history.location.state.__TSR_index;
  if (depth === 0) return open(false);
  return new Promise((resolve) => {
    const stop = history.subscribe(() => {
      stop();
      // Replaced at once, the first step never shows: the router drops a load a newer one takes
      // over. The page in front has let the step back through already.
      if (history.location.state.__TSR_index === 0) void open(true).then(resolve);
      else resolve();
    });
    history.go(-depth);
  });
}

/** Whether going back leads to another page, and not only out of the dialogs open on this one. */
function useCanGoBack() {
  return useRouterState({ select: (s) => s.location.state.__TSR_index > (s.location.state.layers ?? 0) });
}

/** Goes back to where this was opened from, or, opened first as after a restart, `up` instead. */
export function useBack() {
  const router = useRouter();
  const canGoBack = useCanGoBack();
  return (up: () => void) => (canGoBack ? router.history.back() : up());
}

/**
 * The arrow left of a title, which goes back (`useBack`); `up` shows the page a step above in place
 * of this one. Without it, the arrow shows only when there is a step to go back to.
 */
export function BackButton({ up, className }: { up?: () => void; className?: string }) {
  const back = useBack();
  const canGoBack = useCanGoBack();
  if (!canGoBack && !up) return null;
  return (
    <Button variant="ghost" size="icon" aria-label="返回" className={className} onClick={() => back(() => up?.())}>
      <ChevronLeft strokeWidth={1.6} />
    </Button>
  );
}

type Layer = { back: () => void };

/** The open dialogs and panels that are steps in the history, in the order they opened. */
const layers: Layer[] = [];
let layersChanged = () => {};

/**
 * Makes an open dialog or panel a step in the history, as a page opened over this one is: going
 * back over it calls `back`, which closes it or, keeping it open, asks first.
 */
export function useLayerStep(open: boolean, back: () => void) {
  const goneBack = useEffectEvent(back);
  useEffect(() => {
    if (!open) return undefined;
    const layer = { back: () => goneBack() };
    layers.push(layer);
    layersChanged();
    return () => {
      layers.splice(layers.indexOf(layer), 1);
      layersChanged();
    };
  }, [open]);
}

/**
 * Keeps a step in the history for each open layer (`useLayerStep`). Closed by its own means, such
 * as a save, a layer on top takes its step back with it; what the address became meanwhile, such
 * as a school chosen, stays with the page. A step left under a page opened from its layer is passed
 * over when going back or forward. Rendered once, inside the router.
 */
export function LayerSteps() {
  const { history } = useRouter();
  const [, change] = useReducer((n: number) => n + 1, 0);
  // Entries are told apart by their place in the history: one pushed takes the place of all from
  // there on.
  const book = useRef<{
    /** The steps of open layers. */
    steps: { layer: Layer; index: number }[];
    /** The places of steps whose layers have closed. */
    passed: Set<number>;
    /** The address to keep while going back over the steps of closed layers. */
    landing?: string;
  }>({ steps: [], passed: new Set() });
  useEffect(() => {
    const b = book.current;
    layersChanged = change;
    const stop = history.subscribe(({ location, action }) => {
      const index = location.state.__TSR_index;
      if (action.type === "PUSH") for (const i of b.passed) if (i >= index) b.passed.delete(i);
      if (action.type === "PUSH" || action.type === "REPLACE") return;
      if (b.passed.has(index)) {
        // Gone through already on the way here, the page in front lets it pass.
        if (action.type === "FORWARD" || (action.type === "GO" && action.index > 0))
          history.forward({ ignoreBlocker: true });
        else history.back({ ignoreBlocker: true });
        return;
      }
      if (b.landing === undefined) {
        const over = b.steps.filter((s) => s.index > index);
        b.steps = b.steps.filter((s) => s.index <= index);
        for (const s of over.toReversed()) {
          b.passed.add(s.index);
          s.layer.back();
        }
      } else {
        // Replaced at once, as in openSection: the address left behind never shows.
        if (b.landing !== location.href) history.replace(b.landing, location.state, { ignoreBlocker: true });
        b.landing = undefined;
      }
      change();
    });
    return () => {
      stop();
      layersChanged = () => {};
    };
  }, [history]);
  // After each change, once the layers it concerns have rendered, closed or stayed open.
  useEffect(() => {
    const b = book.current;
    if (b.landing !== undefined) return;
    const { location } = history;
    let below = location.state.__TSR_index;
    for (let top = b.steps.at(-1); top?.index === below && !layers.includes(top.layer); top = b.steps.at(-1)) {
      b.steps.pop();
      b.passed.add(below);
      below -= 1;
    }
    for (const s of b.steps) if (!layers.includes(s.layer)) b.passed.add(s.index);
    b.steps = b.steps.filter((s) => layers.includes(s.layer));
    if (below < location.state.__TSR_index) {
      b.landing = location.href;
      // A step only just pushed is in the browser's history before going back over it.
      history.flush();
      history.go(below - location.state.__TSR_index, { ignoreBlocker: true });
      return;
    }
    for (const layer of layers) {
      if (b.steps.some((s) => s.layer === layer)) continue;
      const { href, state } = history.location;
      history.push(href, { ...state, layers: (state.layers ?? 0) + 1 }, { ignoreBlocker: true });
      b.steps.push({ layer, index: history.location.state.__TSR_index });
    }
  });
  return null;
}
