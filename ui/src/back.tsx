import { type AnyRouter, useCanGoBack, useRouter } from "@tanstack/react-router";
import { ChevronLeft } from "lucide-react";
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
