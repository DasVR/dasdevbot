# dasdevbot: missing screens, UX spec v0 (flows + states, no pixels)

Owner: UX Designer · For: Creative Director (clear before pixels), then Design Engineer · 2026-09-30
Written against `DASDEVBOT-LOOK.md` (v1.1 + amendments) and `INTERACTIONS.md` (v2 + CD rulings). No token values, colors or motion are invented here; every visual reference points to an existing rule. Anything that would bend a rule is listed under **Open for CD** at the end.

Build order (DE): 1 approval queue · 2 first run · 3 teammate detail · 4 history · 5 run failure · 6 mode return · 7 settings.

Shared laws every screen inherits
- Colored dot count on screen = count of things that need Arriq. Waiting on you = filled amber dot. Failed/expired = hollow ochre ring. Running/done = ink-3 text, no dot. Idle = nothing.
- Nothing is approved outside the card. Every entry point (queue row, OS banner, tray, island, voice) ends at the card in its thread, where the seen lock, hold and 6s undo run.
- Mic law: no status color on or next to any control that can listen. Counts there are ink text ("1 waiting").
- Flat hairline on paper everywhere below unless a line says glass. Puff = approval card only.
- System loading shows nothing. Teammate work shows that teammate's trace (max 2 looping, rest static).
- Errors: what happened, what is safe, what happens next. No pen, no humor.

---

## 1. Approval queue (the Review sheet + keyboard traversal)

Job: see everything that needs me, in one place, and get to each card fast, without deciding anything blind.
Primary action: open the next waiting card. Deciding happens on the card, never in the list.

Entry points
- Sidebar row "Waiting on you · N" (ink text, amber dot only on the row, not near the composer mic).
- `⌥↓` from anywhere outside a text field: focuses the oldest pending card; if it lives in another teammate's thread, run teammate switch (#21) first, then focus.
- OS notification "Review" (5a mock), tray/menu bar/island dropdown row.
- Returning from Focus or Away (see §6), the sheet opens itself once.

Anatomy (flat hairline sheet, `--r-2xl`, paper; not glass, per CD rule 3)
1. Title "Review" (title 20/620) + one ink-2 summary line: "3 waiting on you · 3 done while you were away" (second half only after a mode return).
2. **Waiting on you** section. One row per pending approval: amber dot · teammate (620) · action title · right side mono `target · risk · HH:MM` (`DasVR/NIL #212 · external · 11:14`). Destructive rows say `destructive` in text; clay never appears in the list (clay lives only in the card's risk strip).
3. **Done** section (only after a mode return): receipt rows, check or dash, mono time + ev id. Read-only.
4. Close (text button) and Esc.

Order
- Waiting rows sort oldest first, the same order `⌥↓` walks, so the list and the shortcut never disagree.
- Expiring (CD ruling 1): a card within 2 min of expiring keeps its place and shows static mono ink-2 "expires 1:52" (no reorder, no countdown). Only the expiring-soon toast breaks through.

Interaction
- Row click / Enter: sheet closes (exit, shorter than enter), the teammate's thread opens, the stream scrolls so the source event sits 18px above the card, card gets focus. Seen lock starts counting **after** the card lands, never while the sheet was open.
- After a decision's 6s undo closes, the receipt's quiet line offers "Next waiting · ⌥↓" if anything is still pending. It never auto-advances (a card must never appear under a hand that is still holding keys).
- No multi-select, no "approve all", no swipe actions. Batch is the one feature this screen must refuse.

States
- **Empty (nothing waiting):** one ink-2 line "Nothing is waiting on you." No pen underline here (it is a sheet, not an empty screen). If opened from `⌥↓` with nothing pending: no sheet, one toast "Nothing is waiting on you."
- **Loading:** nothing drawn until the list is in (system load). Rows arrive whole.
- **Daemon lost:** rows stay visible from the last snapshot, ink-2 line under the title "Showing what was waiting at 11:52. Your decisions are saved." Row click still opens the card, card shows busy (45%, no actions) per #18.
- **Card expired while sheet open:** row sets in place to hollow ochre ring + "Expired · Reviewer will ask again on the next push." and moves to Done on next open.
- **Card decided on another device:** row leaves (FLIP), toast "Decided on <device>: Approved · ev_0143".

Modes
- Here: sheet on demand. Focus: nothing opens it except the user; expiring-soon toast (once, ~2 min out) has one action, "Review". Away: sheet opens once on return.

---

## 2. First run

Job: from install to "Reviewer is watching my repo and will ask before posting" in under a minute, with the trust rules visible before anything runs.
Primary action per step: one button. No tour, no carousel, no feature slides.

Steps (one flat paper screen each; display 28/620 title allowed per tokens; step count "1 of 4" mono ink-3)
1. **Local daemon.** Auto-detects `127.0.0.1:7421` silently. Found: skip the step entirely. Not found: title "dasdevbot runs a small helper on this machine." + one line on what it does + "Start helper" (primary) and "I run it elsewhere" (link to Devices/Advanced address field). Retry states use the #18 banner copy pattern, no spinner.
2. **GitHub.** "Connect GitHub" opens the OS browser (native, not an embedded webview). Waiting state is ink-2 text "Finish in your browser. This window will continue on its own." plus "Open again" link. Return to app on success; on cancel stay here, no error tone.
3. **Repo + branch.** Search list of repos (flat rows, typed match bolded), branch defaults to repo default. One pick for Phase 1. Empty result: "No repo matches “x”." Permission-missing repo row shows ink-3 "needs access" and a link to fix it on GitHub.
4. **The rules, before anything runs.** Title "Everyone asks before it acts." Shows the default rule list with pen checkboxes, all ask-before rules ON: posting to GitHub, writing outside the repo, anything destructive (destructive is shown checked and disabled: "always asks"). Teammates listed under each rule. One line: "Nothing leaves this machine without your OK. You can loosen this later, one rule at a time." Button "Start watching DasVR/NIL".

Land: Reviewer's thread in its empty state (existing #15 + CD amendment: one ink-2 line + one pen underline under the headline, drawn once). Mode is Here.

Deliberately deferred (ask at the moment of need, not in first run)
- OS notification permission (CD ruling 5: one ink-2 line of why before the OS dialog): asked the first time an approval arrives while the window is unfocused, with the in-app card already visible, copy "Want a Review banner when something waits on you?".
- Microphone: asked on first push-to-talk press only.
- Mode explanation: first time the user opens the mode switcher, one ink-2 line under each mode; never a modal.

Recoverability: every step has Back; quitting mid-flow resumes at the same step; nothing runs until step 4 is confirmed.

---

## 3. Teammate detail

Job: understand what one teammate does, what it is allowed to do without asking, and what it has done, and change that safely.
Entry: click the teammate's name in the pane header, or "About" in its sidebar row context menu. Opens as a flat right pane (companion width) over the thread, not a new window; Esc closes and returns focus to where it was.

Anatomy (flat hairline sections, paper, no puff)
1. Identity: monogram disc, name (title), one-line job ("Reviews every push and asks before posting."), current state line:
   - running: trace + mono `lease rev_01 · 2m14s · 208 / 8000 tok`
   - waiting on you: amber dot + "1 waiting" + link "Open card" (goes to card, focus)
   - failed: hollow ochre ring + ink "Last run failed" + mono `exit 101` + link "Open run" (§5)
   - paused: pen lifted, ink-2 "Paused. Pending cards still wait for you."
   - idle: nothing (no status word beyond "last ran 10:43")
2. **Wakes on:** triggers in mono (`repo.push · DasVR/NIL · phase0`).
3. **Asks before:** the rules that apply to this teammate, pen checkboxes, same objects as the sidebar Rules (edit one place, both update).
4. **Budget:** today's spend and cap in mono, from Ledger. Numbers never animate.
5. **Recent runs:** last 10 receipts/runs as flat rows (mark · outcome · title · mono time + ev id). Row opens the thread scrolled to that event. "All history" link filters History (§4) to this teammate.
6. Footer: "Pause Reviewer" switch (#13 behavior, scoped to one teammate).

Trust rule for editing
- **Tightening** a rule (checking an ask-before) applies at once, toast "Reviewer now asks before posting to GitHub."
- **Loosening** (unchecking) an ask-before for an external rule is itself a consequential action: the checkbox does not uncheck on click; it opens an inline confirm row in place (same no-height-jump pattern as deny #4): "Reviewer will post to GitHub without asking." [Keep asking] [Stop asking], equal weight, Stop asking needs the 600ms hold, then 6s undo. Destructive rules cannot be loosened.

States: loading = nothing until data lands (runs list) or Ledger's reading trace if Ledger is fetching history (#22). Never run: sections 1–4 shown, section 5 reads "No runs yet. Reviewer wakes on the next push."

---

## 4. History

Job: find what was decided, by whom, when, and what it did; prove nothing left the machine unexpectedly.
Entry: sidebar "History N". Full pane (replaces thread), flat paper list.

- Rows = receipts, newest first: mark · outcome word (Approved moss / Denied ink / Expired ochre) · teammate · title · mono time + ev id · "posted" or "nothing posted" ink-3.
- Filters as text toggles, no chips with color: teammate, outcome, risk, date. Search matches title, target, and ev ids (pasting `ev_0143` jumps straight to it).
- Row opens receipt detail inline (disclosure, #23 behavior): evidence dl, draft as posted, reason if denied, decision device, link "Open in thread".
- Undo is **not** offered here; once filed, a receipt is final (matches the 6s window rule).
- Empty: "No decisions yet. Receipts file here after each approval." Filtered empty: "No receipts match these filters." + "Clear filters".
- Loading: nothing, rows arrive whole with the 90ms stagger only on first open per session.

---

## 5. Run failure detail

Job: know what failed, whether anything was affected, and what to do.
Entry: failed teammate row, teammate detail "Open run", OS banner (failures are allowed to notify).
- Opens in the thread at the failed event, which is a flat row: hollow ochre ring + ink sentence ("Builder's tests failed on phase0.") + mono `exit 101 · 41 passed, 2 failed`.
- Disclosure shows the last 40 log lines in mono on a sunken well; "Copy log" (toast "Copied log").
- Reassurance line always present: "Nothing was posted. No pending decision was lost."
- Actions: "Retry run" and "Ask Builder why" (prefills the composer, does not send). Retry of anything external still goes through an approval card.
- Clears its dot when the user opens it (seen), not when it is retried; the count of dots stays honest.

---

## 6. Mode return (Focus / Away → Here)

Job: re-enter without hunting. Built to `03-modes` mock.
- Switching mode: label + line glyph in the header, no color. Away shows one ink-2 line under the switcher on first use: "Teammates keep working. They still stop and wait at every approval."
- While in Focus: all banners held except expiring approvals (once, ~2 min out, one toast, action "Review"). Destructive never breaks through.
- On return to Here: the Review sheet (§1) opens once with the "Welcome back" summary and the Done section. If nothing is waiting and nothing was done: no sheet, no toast.
- Returning because the user clicked an OS "Review" banner: skip the sheet, go straight to that card.

---

## 7. Settings (structure only)
Flat hairline sections, native window controls.
- **General:** default mode, launch at login, window style (full / companion / pill).
- **Approvals:** hold time slider 400–1500ms (cannot go below 400; destructive is always 2× approve, min 1200ms), undo window display (6s, read-only in Phase 1).
- **Notifications:** asks / failures / expiring (calm states have no toggle because they never notify). Preview shows the one-action "Review" banner.
- **Voice:** push-to-talk (default) vs hold-free, read-aloud before voice approve (on, locked). Line: "Voice never approves destructive actions."
- **Devices / Advanced:** daemon address, connected devices, reset first run.
- Rules live on teammates and the sidebar, not here (one source).

---

## Open for CD (need a ruling before pixels)
1. Expiring-soon card jumps to the top of the queue with static "expires 1:52" text. OK, or keep strict oldest-first?
2. Loosening an external ask-before rule requires the 600ms hold + undo (§3). I think this is the most important trust gap in the current mocks. Confirm.
3. Teammate detail as a flat companion-width right pane over the thread, not a new route/window.
4. Receipt quiet line "Next waiting · ⌥↓" after undo closes, never auto-advance.
5. Notification and mic permission deferred to first need, not first run.
6. History has no undo; filed means final.

## CD rulings (2026-09-30), SCREENS v0 cleared for pixels
1. The queue stays strictly oldest first, and ⌥↓ steps in the same order. An expiring row stays where it is and shows static mono ink-2 text, "expires 1:52", with no reordering and no countdown. The expiring-soon toast is the only thing that breaks through.
2. Confirmed. Loosening an external rule needs the 600ms hold plus the 6s undo. Destructive rules can never be loosened, and tightening applies right away.
3. Confirmed. Teammate detail is a flat pane at companion width over the thread. Esc returns focus to where it was.
4. Confirmed. The quiet line "Next waiting · ⌥↓" appears after undo closes, and the queue never auto-advances.
5. Confirmed. Permissions are asked at first need, each with one ink-2 line explaining why, shown before the OS dialog.
6. Confirmed. Filed means final, and History is a read-only record. Copy on filed items: "Filed. This can't be changed from here."

## Phase 1 overrides (Security PERMISSIONS.md v1.2, confirmed by CD 2026-09-30). These outrank the sections above for Phase 1, which is Windows only.
- **C1, destructive actions are denied and never asked.** The queue never has a destructive row. The thread shows a flat ink row with a dash and no dot, reading "Destructive actions are off in this build.", with the mono command under it. It doesn't notify and doesn't count as waiting. In §3 the destructive rules are shown checked and disabled with the note "off in this build". In §7 the settings keep only the external hold time, from 400 to 1500ms.
- **C6, voice never approves anything.** In §7 the "read-aloud before voice approve" setting is removed. Saying "approve" gets the ink-2 reply "Approve on screen." It's the same in queue rows: voice can open the Review sheet, and that's all.
- **C4, Windows Hello confirms every external approval.** The order is seen lock, then the 600ms hold, then Hello, then the 6s undo. While Hello is open, the card stays glass and shows "Confirm with Windows Hello". Cancelling returns it to waiting with no error tone, and its row stays in the queue in place. If the queue is open underneath, it doesn't change.
- **Loosening a rule (§3):** it's the 600ms hold, then Hello, then the 6s undo, because giving a teammate permission is at least as consequential as a single approval. Confirmed by CD 2026-09-30. It uses the same "Confirm with Windows Hello" line.
- **Windows only:** first run's GitHub step opens the default browser. The island and menu bar entry points are out. The tray dropdown stays, with an ink numeral badge on the icon.

## Acceptance checks for the queue and first-run PRs (flow, not pixels)
Every state listed below needs a capture. Each check is a PASS or FAIL.

**Approval queue**
1. Rows run oldest first, and ⌥↓ steps through the same order. An expiring row stays in place and shows a static "expires m:ss" label.
2. Opening a row closes the sheet, switches teammate if needed, and focuses the card. The seen lock starts once the card lands, not while the sheet is open.
3. No batch approve, no multi-select, and nothing auto-advances. After undo closes, the receipt shows "Next waiting · ⌥↓".
4. Empty queue from the sidebar shows one line. Empty queue from ⌥↓ shows a toast and no sheet.
5. When the daemon drops, the last snapshot stays up with the "Your decisions are saved" line, and the card goes busy.
6. When a card expires while the sheet is open, its row sets in place to a hollow ochre ring.
7. Cancelling Windows Hello puts the card back to waiting. Its row doesn't move, and there's no error tone.
8. No destructive rows ever appear (C1).
9. The number of colored dots equals the number of items waiting. The tray icon uses an ink numeral with no status color.

**First run**
1. When the daemon is found, its step is skipped entirely, and nothing is drawn while checking.
2. GitHub opens in the default browser. Waiting shows its line of copy. Cancelling stays on the step with no error tone.
3. The rules step comes before anything runs, with the ask-before rules on. Destructive shows as disabled with "off in this build".
4. It lands on Reviewer's empty state, with one line and one underline drawn once, in Here mode.
5. No notification or mic prompt appears during first run. Each one comes at first need, after one line explaining why.
6. Back works on every step, and quitting resumes at the same step.

## Mascot (pen mark #8, CD rulings 2026-09-30, welcome studies passed). This overrides §2 and the first-run checks above.
- **Character:** B "Curl". The curl stays open and a little off-center, so it reads as the pen lifting off the page.
- **First run:** there's a fixed 48px slot to the left of every step title, at the same x and y on every step. Curl draws once, the first time it appears, and stays in the slot across steps. On each step change it gives one "look" toward the new title. Back gets the same single look, never a redraw. With reduced motion, Curl appears already drawn and doesn't look.
- **Helper skip (kept):** when the helper is found, step 1 is skipped and step 2 shows one ink-3 line: "Helper found on this machine, step 1 skipped · 127.0.0.1:7421". Keeping it tells the user why step 1 never appeared and where the helper is, at no cost.
- **Landing:** Reviewer's pane has Curl in the slot beside "Reviewer reads every push to DasVR/dasdevbot and asks before it posts to GitHub." The underline draws once, after Curl lands. Curl then blinks about every 6s (CD-approved idle). With reduced motion it's static. The blink stops the moment a teammate trace is running on screen, so the motion budget stays at one moving thing.
- **Daily greeting:** the first open of each day shows a greeting line at the top of the current pane, with Curl beside it. **Curl stays when items are waiting**, and the waiting count becomes the ink-2 line. It leaves on the first send, teammate switch, or Review open, and doesn't come back that day. Returning from Focus or Away shows the Review sheet, not the greeting.
- **Never:** in the thread, in other empty states, on cards or receipts, in the queue, or in OS surfaces (tray, toasts, notifications).

### Cleared copy
- Repo step: "Reviewer reads your pushes there. It asks before it posts anything." / "Reviewer watches one repo in this build."
- GitHub connected: "Connected as arriq"
- Landing: "Reviewer reads every push to DasVR/dasdevbot and asks before it posts to GitHub."
- Afternoon greeting: "1 waiting on you · 4 done today."
- Evening greeting: "Evening, Arriq." / "Nothing waiting · 6 done today."

### Added checks
- First run 7: the slot is at the same x and y on every step, at 2x.
- First run 8: Curl draws exactly once in the whole flow, with one look per step change.
- First run 9: the helper-skip line appears only when step 1 was skipped.
- Landing 1: the blink is about 6s, static with reduced motion, and paused while any trace runs.
- Greeting 1: it shows once per day, and when something is waiting the waiting count replaces the ink-2 line.
