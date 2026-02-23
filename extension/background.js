// Attend Browser Bridge: background script
//
// Receives selection messages from the content script and relays them
// to the attend native messaging host via one-shot sendNativeMessage.

browser.runtime.onMessage.addListener((msg) => {
  if (msg.type !== "selection") return;

  browser.runtime
    .sendNativeMessage("attend", {
      text: msg.text,
      url: msg.url,
      title: msg.title,
      is_code: msg.is_code,
    })
    .catch((err) => {
      // Native host not installed or not running: silently ignore.
      // This is expected when attend is not active.
      console.debug("attend: native messaging failed:", err.message);
    });
});
