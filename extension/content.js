// Attend Browser Bridge: content script
//
// Observes text selections on the page and sends the selected HTML to
// the background script for relay to the attend native messaging host.
// The Rust bridge converts HTML to markdown using htmd.

let debounceTimer = null;
const DEBOUNCE_MS = 300;

document.addEventListener("selectionchange", () => {
  clearTimeout(debounceTimer);
  debounceTimer = setTimeout(onSelectionStable, DEBOUNCE_MS);
});

function onSelectionStable() {
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

  browser.runtime.sendMessage({
    type: "selection",
    html: html,
    url: location.href,
    title: document.title,
  });
}
