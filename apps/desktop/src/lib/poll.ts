/**
 * Snapshot polling for a window.
 * - One request in flight at a time: the next poll is scheduled only after the
 *   previous one settles, so a slow daemon never stacks overlapping calls.
 * - A hidden window does not poll. Tauri keeps the card, settings and voice
 *   webviews alive while hidden, and each poll used to cost a daemon round
 *   trip plus a full snapshot through IPC. Becoming visible refreshes at once.
 */
export function startPolling(refresh: () => Promise<void>, everyMs: number): () => void {
  let stopped = false;
  let running = false;
  let timer: ReturnType<typeof setTimeout> | undefined;

  const hidden = () => document.visibilityState === "hidden";

  const schedule = () => {
    clearTimeout(timer);
    if (!stopped && !hidden()) {
      timer = setTimeout(() => void run(), everyMs);
    }
  };

  const run = async () => {
    if (stopped || running) {
      return;
    }
    running = true;
    try {
      await refresh();
    } finally {
      running = false;
      schedule();
    }
  };

  const onVisibility = () => {
    if (!hidden()) {
      clearTimeout(timer);
      void run();
    }
  };

  document.addEventListener("visibilitychange", onVisibility);
  void run();
  return () => {
    stopped = true;
    clearTimeout(timer);
    document.removeEventListener("visibilitychange", onVisibility);
  };
}
