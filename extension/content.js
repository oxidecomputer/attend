// Attend Browser Bridge: content script
//
// Captures text selections on the page and sends the selected HTML to
// the background script for relay to the attend native messaging host.
// The Rust bridge converts HTML to markdown using htmd.
//
// Uses mouseup (for drag selections) and keyup (for keyboard selections
// with Shift+arrows) to avoid the continuous firing of selectionchange.

let lastSentHtml = "";

document.addEventListener("mouseup", onSelectionComplete);
document.addEventListener("keyup", (e) => {
  // Only check on key events that could modify a selection (Shift+arrows, etc.)
  if (e.shiftKey || e.key === "Escape") {
    onSelectionComplete();
  }
});

function onSelectionComplete() {
  const sel = window.getSelection();
  if (!sel || sel.isCollapsed) return;

  const text = sel.toString().trim();
  if (!text) return;

  // Serialize the selection as an HTML fragment.
  const range = sel.getRangeAt(0);
  const fragment = range.cloneContents();
  const wrapper = document.createElement("div");
  wrapper.appendChild(fragment);
  const html = wrapper.innerHTML;

  // Deduplicate: don't resend the exact same selection.
  if (html === lastSentHtml) return;
  lastSentHtml = html;

  browser.runtime.sendMessage({
    type: "selection",
    html: html,
    url: location.href,
    title: document.title,
  });
}
