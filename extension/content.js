// Attend Browser Bridge: content script
//
// Observes text selections on the page and sends them to the background
// script for relay to the attend native messaging host.

let debounceTimer = null;
const DEBOUNCE_MS = 300;

document.addEventListener("selectionchange", () => {
  clearTimeout(debounceTimer);
  debounceTimer = setTimeout(onSelectionStable, DEBOUNCE_MS);
});

function onSelectionStable() {
  const sel = window.getSelection();
  const text = sel ? sel.toString().trim() : "";

  if (!text) return;

  // Detect whether the selection is inside a code block.
  let isCode = false;
  if (sel.rangeCount > 0) {
    const range = sel.getRangeAt(0);
    const ancestor = range.commonAncestorContainer;
    const el =
      ancestor.nodeType === Node.ELEMENT_NODE
        ? ancestor
        : ancestor.parentElement;
    if (el && el.closest("code, pre, .highlight, .code")) {
      isCode = true;
    }
  }

  browser.runtime.sendMessage({
    type: "selection",
    text: text,
    url: location.href,
    title: document.title,
    is_code: isCode,
  });
}
