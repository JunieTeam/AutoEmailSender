import { type AnyRouter, useRouter, useRouterState } from "@tanstack/react-router";
import { ChevronLeft } from "lucide-react";
import { useEffect, useEffectEvent, useState } from "react";
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

/** How to answer whether to throw away what was typed in a layer, once going back would close it. */
export type Asking = { stay: () => void; leave: () => void };

type Layer = {
  /** Closes it, as going back over its step does, unless it cannot close now. */
  back: () => void;
  /** Whether going back may close it: with unsaved input, once the user has said. */
  leave: () => Promise<boolean>;
};

/** The open dialogs and panels that are steps in the history, in the order they opened. */
const layers: Layer[] = [];
let layersChanged = () => {};

/**
 * Makes an open dialog or panel a step in the history, as a page opened over this one is: going
 * back over it calls `back`, which closes it. With `unsaved` input, going back past it first asks
 * whether to throw that away, and waits; while it asks, this returns how to answer.
 */
export function useLayerStep(open: boolean, back: () => void, unsaved = false): Asking | null {
  const goneBack = useEffectEvent(back);
  const holding = useEffectEvent(() => unsaved);
  const [asking, setAsking] = useState<Asking | null>(null);
  useEffect(() => {
    if (!open) return undefined;
    let answer: ((leave: boolean) => void) | undefined;
    const layer = {
      back: () => goneBack(),
      leave: () =>
        holding()
          ? new Promise<boolean>((resolve) => {
              answer = (leave) => {
                setAsking(null);
                resolve(leave);
              };
              setAsking({ stay: () => answer?.(false), leave: () => answer?.(true) });
            })
          : Promise.resolve(true),
    };
    layers.push(layer);
    layersChanged();
    return () => {
      // Closed while asking, as by its close button, it no longer holds going back.
      answer?.(true);
      layers.splice(layers.indexOf(layer), 1);
      layersChanged();
    };
  }, [open]);
  return asking;
}

/**
 * Keeps a step in the history for each open layer (`useLayerStep`). Closed by its own means, such
 * as a save, a layer on top takes its step back with it; what the address became meanwhile, such
 * as a school chosen, stays with the page. A step left under a page opened from its layer is passed
 * over when going back or forward. Rendered once, inside the router, which keeps the browser's
 * history.
 */
export function LayerSteps() {
  const { history } = useRouter();
  useEffect(() => {
    // Entries are told apart by their place in the history: one pushed takes the place of all from
    // there on, so the last pushed is the last there is.
    /** The steps of open layers. */
    let steps: { layer: Layer; index: number }[] = [];
    /** The places of steps whose layers have closed. */
    const passed = new Set<number>();
    let last = history.location.state.__TSR_index;
    /** The address to keep while going back over the steps of closed layers. */
    let landing: string | undefined;
    // After the task that changed something: a page a closing layer opens comes once the router
    // has checked with whatever holds navigation, and then takes the place of the layer's step.
    let timer: ReturnType<typeof setTimeout> | undefined;
    const schedule = () => {
      timer ??= setTimeout(() => {
        timer = undefined;
        sync();
      });
    };
    function sync() {
      if (landing !== undefined) return;
      const { location } = history;
      // While a going back or forward waits, as for a page that saves first or a layer that asks,
      // the browser is already where the router is not yet; it is left to finish.
      if (browserIndex() !== location.state.__TSR_index) return;
      let below = location.state.__TSR_index;
      for (let top = steps.at(-1); top?.index === below && !layers.includes(top.layer); top = steps.at(-1)) {
        steps.pop();
        passed.add(below);
        below -= 1;
      }
      for (const s of steps) if (!layers.includes(s.layer)) passed.add(s.index);
      steps = steps.filter((s) => layers.includes(s.layer));
      if (below < location.state.__TSR_index) {
        landing = location.href;
        history.go(below - location.state.__TSR_index, { ignoreBlocker: true });
        return;
      }
      for (const layer of layers) {
        if (steps.some((s) => s.layer === layer)) continue;
        const { href, state } = history.location;
        history.push(href, { ...state, layers: (state.layers ?? 0) + 1 }, { ignoreBlocker: true });
        // Its own entry: pushes left to the browser together become one.
        history.flush();
        steps.push({ layer, index: history.location.state.__TSR_index });
      }
    }
    const stop = history.subscribe(({ location, action }) => {
      const index = location.state.__TSR_index;
      if (action.type === "PUSH") {
        last = index;
        for (const i of passed) if (i >= index) passed.delete(i);
      }
      if (action.type !== "PUSH" && action.type !== "REPLACE") {
        if (passed.has(index)) {
          // Gone through already on the way here, the page in front lets it pass; going forward to
          // the last entry, it turns back.
          const forward = action.type === "FORWARD" || (action.type === "GO" && action.index > 0);
          if (forward && index < last) history.forward({ ignoreBlocker: true });
          else history.back({ ignoreBlocker: true });
          return;
        }
        if (landing === undefined) {
          const over = steps.filter((s) => s.index > index);
          steps = steps.filter((s) => s.index <= index);
          for (const s of over.toReversed()) {
            passed.add(s.index);
            if (layers.includes(s.layer)) s.layer.back();
          }
        } else {
          // Replaced at once, as in openSection: the address left behind never shows.
          if (landing !== location.href) history.replace(landing, location.state, { ignoreBlocker: true });
          landing = undefined;
        }
      }
      schedule();
    });
    const unblock = history.block({
      // Going back past layers waits for each holding unsaved input to say whether to throw it away.
      blockerFn: async ({ nextLocation, action }) => {
        if (action === "PUSH" || action === "REPLACE") return false;
        const to = nextLocation.state.__TSR_index;
        for (const s of steps.toReversed())
          if (s.index > to && layers.includes(s.layer) && !(await s.layer.leave())) return true;
        return false;
      },
      enableBeforeUnload: false,
    });
    layersChanged = schedule;
    // The router undoing a going back it held tells no one once the browser is back.
    window.addEventListener("popstate", schedule);
    // Layers open before this.
    schedule();
    return () => {
      clearTimeout(timer);
      stop();
      unblock();
      window.removeEventListener("popstate", schedule);
      layersChanged = () => {};
    };
  }, [history]);
  return null;
}

/** Where the browser is in its history, as the router numbers its entries. */
function browserIndex(): unknown {
  const state: unknown = window.history.state;
  return typeof state === "object" && state !== null && "__TSR_index" in state ? state.__TSR_index : undefined;
}
