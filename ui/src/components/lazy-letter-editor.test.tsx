// @vitest-environment happy-dom
import { act, StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { LazyLetterEditor } from "./lazy-letter-editor";

// The owner stops asking once told the focus is taken, as the reply box does. The first load
// suspends, so the editor shown is not the first one created.
function Asking() {
  const [focus, setFocus] = useState(true);
  return (
    <LazyLetterEditor
      id="letter"
      markdown="王老师："
      placeholders={[]}
      focus={focus}
      onFocused={() => setFocus(false)}
      disabled={false}
      onChange={() => {}}
    />
  );
}

it("takes the focus asked for when it is first loaded, under StrictMode as in the app", async () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () =>
      root.render(
        <StrictMode>
          <Asking />
        </StrictMode>,
      ),
    );
    await vi.waitFor(() => expect(document.activeElement?.closest(".tiptap")).not.toBeNull());
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
});
