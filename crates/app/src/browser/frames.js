// The page as it shows: its document, with the visible frames of its own site in it, as one a
// script writes the page's text into. Another site's frame cannot be read; a hidden one, as one
// holding a page for printing, shows nothing of the page.
const shows = (frame) =>
  Boolean(frame.contentDocument?.body) &&
  frame.getClientRects().length > 0 &&
  frame.ownerDocument.defaultView.getComputedStyle(frame).visibility !== "hidden";
const documents = (page) => [
  page,
  ...[...page.querySelectorAll("iframe")]
    .filter(shows)
    .flatMap((frame) => documents(frame.contentDocument)),
];
