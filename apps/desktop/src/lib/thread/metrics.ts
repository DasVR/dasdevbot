/**
 * Lengths the thread prototype locks that tokens.css does not name.
 * Components read these through CSS variables. They are not colors.
 */

/** Centered thread column. */
export const THREAD_WIDTH = "720px";

/** Monogram (20) plus the 8px gap before a bubble. Aligns tools, cards, receipts. */
export const GUTTER = "28px";

/** User bubble. No tail. */
export const USER_BUBBLE_MAX = "480px";

/** Bot bubble. */
export const BOT_BUBBLE_MAX = "600px";

/** Approval slot. Same width the card was drawn at. */
export const SLOT_WIDTH = "520px";

/** Live tool column. */
export const TOOLS_WIDTH = "560px";

/** Top bar and the composer pill. No size token is 60. */
export const BAR_HEIGHT = "60px";

/** Fade the thread under the top bar. No space token is 64. */
export const MASK_FADE = "64px";

/** Composer sits off the bottom edge. */
export const COMPOSER_INSET = "22px";

/** Thread column top inset. tokens.css has no 40px space. */
export const THREAD_PAD_TOP = "40px";

/** Room so the last row clears the composer. */
export const THREAD_PAD_BOTTOM = "124px";

/** User bubble to its time. tokens.css --s-1 is 4; the row is 5. */
export const USER_ROW_GAP = "5px";

/** Wordmark disc. Off the 12.5px meta ramp. */
export const DISC_SIZE = "13px";

/** Space between thread blocks. No space token is 22. */
export const THREAD_GAP = "22px";

/** Brand disc corner. Nested under the 26px disc; --r-sm is 10. */
export const DISC_RADIUS = "9px";

/** Thread monogram. The card monogram stays 32. */
export const TURN_MARK = "20px";

/** Pressed ink fill. tokens.css has no --ink-press. LOOK Phase 1 names this for fills only. */
export const INK_PRESS = "#1E1B18";
