import init from "/lili-ui.js";

await init({ module_or_path: "/lili-ui_bg.wasm" });
window.__LILI_HYDRATED__ = true;

if (window.location.pathname === "/notifications") {
  const installNotificationSizing = () => {
    if (window.__LILI_NOTIFICATION_SIZE_OBSERVER__) return;
    const stack = document.querySelector(".notification-stack");
    if (!stack) {
      window.requestAnimationFrame(installNotificationSizing);
      return;
    }
    let lastHeight = null;
    const synchronizeHeight = () => {
      const invoke = window.__TAURI_INTERNALS__?.invoke;
      if (!invoke) {
        lastHeight = null;
        return;
      }
      const height = Math.min(158, Math.max(16, Math.ceil(stack.scrollHeight) + 8));
      if (height === lastHeight) return;
      lastHeight = height;
      void invoke("resize_notification_window", { height }).catch(() => {
        lastHeight = null;
      });
    };
    const observer = new ResizeObserver(synchronizeHeight);
    observer.observe(stack);
    window.__LILI_NOTIFICATION_SIZE_OBSERVER__ = observer;
    synchronizeHeight();
  };
  installNotificationSizing();
}
