# DECISIONS.md: dasdevbot rulings log

Every settled ruling goes here (plan v2, rule 5). Reversing a logged ruling needs Arriq's okay.
Settlers: CD for design (rule 2), SD for security. Format: id · date · settler · ruling · source.
IDs: `D-CD-###` = Creative Director. `D-SD-###` = **Security Director**. `D-SDIR-###` = **Studio Director** (process and scope). "SD" in this file always means Security Director. Entries are never deleted or rewritten. To reverse one, add a new entry that names the old id and Arriq's okay.
This file on `main` in DasVR/dasdevbot is the only official copy.

## Design (settler: Creative Director)

### D-CD-001 · 2026-10-08 · CD · Approval undo lives on the glass card
After the hold (and Windows Hello under C4), the approval card stays glass for the full 6s undo window. It shows one pen check, the Approve button reads "✓ Approved" in ink, and a paper "Undo Ns" button sits beside it, counting down. The quiet line reads "Posts when undo closes. Nothing is posted yet." Ctrl/⌘+Z restores "Approve draft", clears the ink, refocuses the card and re-arms the seen lock.
The fold to a paper receipt runs only when the window closes: paper for 360ms, then receipt height and slot over 520ms. Reduced motion uses a 160ms crossfade. The filed receipt has no Undo, because filed means final.
Source: `02-approval/approval-hold-undo-file.mp4`, plus LOOK §8.3 and INTERACTIONS S3/S5 as amended 2026-10-08. Passed in #24 at `a7662f9`.

### D-CD-002 · 2026-10-08 · CD · #24 parity, declared items
- Ranked P1 gap: "03-modes not built, due by Oct 14". The titlebar shows nothing the mock lacks: no placeholder and no disabled mode control.
- Declared deviation: drawn Windows caption buttons at native metrics (Segoe Fluent glyphs, Win11 hover and press fills, close #C42B1C, working Snap Layouts flyout). Reason: OS chrome, Windows-only Phase 1. The flyout is pending a check on Arriq's PC.
- Declared deviation: the Builder roster sub drops the lease segment and reads "2m14s · 208 / 8000 tok" in the video's two-line layout.
- C1 demo: the scripted force-push routes through Builder and shows the flat C1 row. There is no visible simulator button.

### D-CD-003 · 2026-09-30 · CD · Standing look rules (carried forward)
For shipped screens, the mock videos outrank the HTML mocks, and the Security Phase 1 waivers (C1, C4, C6) outrank both. For the Today lobe, the CD spec is the reference, and the security waivers still outrank it. Detailed rulings stay in the local look reference set (DASDEVBOT-LOOK.md, INTERACTIONS.md, SCREENS.md, tokens.css).

### D-CD-004 · 2026-10-08 · CD · Today lobe spec v0 rulings (`/workspace/plan/today-lobe/SPEC.md`)
- Type: Figtree + JetBrains Mono per `tokens.css`. Plus Jakarta Sans was NIL only.
- Empty day: "Empty is a receipt". The empty line names what was checked, e.g. "Checked 3 Pocket recordings and your calendar at 11:02 PM." All three sections stay visible when empty.
- Calendar free span as a plan row: NO. A "Next on your calendar: Fri 9:00" line on an empty day: YES.
- Section headings: plain heading plus mono count ("Your plan 5"). Sentence headings: NO, because they read as filler.
- "Held by the floor" folds to one summary line unless a row in it is waiting on Arriq: YES.
- A "now" line in the plan that steps each minute with no animation: YES.
- One-tap "done" stays parked (not free). The M4 "who was right?" slot is reserved as flat rows.
- No greeting or mascot in the lobe v0. Arriq gets asked separately whether Curl's daily greeting moves here.
- Approvals stay under "Held by the floor" until the Oct 12–14 digest test. After that, a move to "Needs you" is a filter change only.

### D-CD-005 · 2026-10-08 · CD · UX Today-flow calls
- The Today spec is `/workspace/plan/today-lobe/SPEC.md`. The digest layout (your plan / needs you / held by the floor) is the spec layout, so TODAY-FLOWS aligns to the spec.
- Changed times: a time, number or id is one token. It swaps instantly with no motion and gets a static note ("moved from 9:00"). Only the surrounding words animate.
- No greeting or Curl in the lobe v0 (pending Arriq).
- Login/2FA handover is a flat hairline row with an amber dot (needs Arriq). No glass, no puff. It opens the desktop handover.
- The M4 "who was right?" row offers three choices, "Laya" / "What ran" / "Can't tell", with no dot. This supersedes "Rules" in SPEC v0.
- Task plans say "steps". The word "plan" is reserved for the Today plan.
- Docs: LOOK §8.5 and INTERACTIONS #9 now show the undo on the glass card, per D-CD-001.

### D-CD-006 · 2026-10-08 · CD · UX conflicts C-3 to C-12 and the stop card
- C-3 YES: no warning color on a plan older than 12h. The reason goes only in the expanded step-line detail.
- C-5 YES: the handover row reads "Open sign-in →" and opens the sign-in panel in that task's thread.
- C-6 YES: one row per source. The amber handover row replaces the ochre "Reconnect in Settings" row.
- C-7 DONE: SPEC G2, §6.1 and §6.2 are calendar-only. Settings shows Pocket as "Comes later" (D-SD-006).
- C-8 YES: the folded "Held by the floor" line names the budget queue and reset time. Expanding it shows "Run on this PC instead" (D-SD-008). No dot, and it doesn't force the section open.
- C-9 YES, once M1 sets the budget. Until then, keep the "1 of 20" count.
- C-10 YES: `laya|actual|cant_tell`. The UI words stay "Laya" / "What ran" / "Can't tell".
- C-11 YES: the model-failed line adds "Nothing was sent to the cloud instead."
- C-12 YES: `plan.*` carries `scope: today|task`, and "defer" becomes `plan.delta`.
- Stop card (D-SD-007): the check retracts over 160ms, then the card shows a single line, "The stop cancelled this post. Nothing was posted.", with no buttons and no dot. It then runs the normal S3 fold to a receipt with a dash and "Stopped" in ink. Reduced motion: instant swap, then a 160ms crossfade. "Cancelled" is rejected as the receipt word because the event is logged as stopped.

### D-CD-007 · 2026-10-08 · CD · #24 passed at `a7662f9`; the remaining nits are tickets
CD PASSed #24 at `a7662f9`. UI Designer's reduced-motion fail and the remaining nits are filed as tickets, not as a fix round (rule 4), all labeled `parity-nit`:
- #37: deny layout (Deny reads "Denied", Approve carries the Undo countdown; mock 2a `commit()` :780–781)
- #38: reduced-motion filing drops the card in one frame (fade over 160ms)
- #39: "Undo 0s" at 45% opacity for a frame while filing (hold "Undo 1s")
- #40: +2.44px height bump on the first paperize frame
- #41: Ctrl/⌘+Z ink retract timing (160ms; instant under reduced motion)
- #42: Snap Layouts flyout, pending Arriq's Windows check
- #43: 03-modes switcher (P1 gap, due Oct 14)

The #24 deviations declared in D-CD-002 stand:
- Drawn Windows caption buttons at native metrics, OS chrome, Windows-only Phase 1.
- Builder sub: lease ids are kept out of the DOM (UI Designer #24 item 7), so the lease segment is dropped.
_Numbered D-CD-004 in PR #44; renumbered D-CD-007 on 2026-10-08 because CD had already logged D-CD-004 and D-CD-005 (Today lobe). Text unchanged._

## Security (settler: Security Director)

### D-SD-001 · 2026-10-08 · SD · #24's CSP and signed-IPC Highs are closed; #24 merged at `a7662f9`
SD confirmed the CSP High and the signed-IPC High CLOSED at `a7662f9`. Decisions are signed card-window Tauri IPC only, and HTTP decide and undo stay 403. #24 merged at exactly that head, in merge commit `7ce1f870313148afc09ea976f554cddcb556d9fd`.

### D-SD-002 · 2026-10-08 · SD · #36's H1/M2/M3 gate G1
H1 (the bearer reaches JS), M2 (unauthenticated calls to the local daemon) and M3 (no Job Object) gate G1, the Oct 16 non-mock Windows run, and each fix needs SD's review first. They did not gate the #24 merge. Arriq's Oct 1 demo waiver ends when a real provider runs on Windows. M1 (NSIS) must land before any installer goes to the demo drop folder. The undo-crash item and the other follow-ups stay as tickets. Tracked in #36.

### D-SD-003 · 2026-10-08 · SD · Handover: where Arriq signs in, and who writes the prompt
- Handover text and URLs come only from fixed per-provider daemon templates with allowlisted URLs. The model never writes either one.
- Browser sign-in opens the default browser, never a webview.
- Claude CLI re-login: Arriq runs it himself. The daemon never spawns it, never reads its pty, and never uses `setup-token`. Afterward the daemon only re-checks auth and the pinned version.
- The Ollama Cloud key is entered only through masked secure input and is pinned to `ollama.com:443`.
_Wording corrected by SD 2026-10-08 20:40 ET._
Source: SD message to UX, 2026-10-08 20:36 ET (relayed by the parent agent).
Affects: TODAY-FLOWS §7.1, §7.3, §7.7; SPEC.md §6.5.

### D-SD-004 · 2026-10-08 · SD · Handover timeout
The default timeout is 10 minutes, configurable up to 30. On timeout the task stays paused or queued and stays in "needs you". It is never cancelled and never downgraded.
Source: SD message to UX, 2026-10-08 20:36 ET (relayed by the parent agent).
Affects: TODAY-FLOWS §7.5; MECHANICS-SPEC §1.

### D-SD-005 · 2026-10-08 · SD · Re-auth with a wider scope or a new account
A re-auth that asks for a wider scope, or that uses a different account, goes through the approval card, the hold and Windows Hello. The daemon compares the granted scopes against the previous grant. The daemon compares the scopes, never the model. A same-or-narrower re-auth needs no card.
_Wording corrected by SD 2026-10-08 20:40 ET._
Source: SD message to UX, 2026-10-08 20:36 ET (relayed by the parent agent).
Affects: TODAY-FLOWS §7.4, §7.7.

### D-SD-006 · 2026-10-08 · SD · Pocket is out of G2
Pocket ingest is not in G2. It stays Phase 2 behind the R6 consent flow and needs SD review before any ingest.
_Wording corrected by SD 2026-10-08 20:40 ET._
Source: SD message to UX, 2026-10-08 20:36 ET (relayed by the parent agent).
Affects: TODAY-FLOWS §1, §3; SPEC.md §1 (G2 row), §6.1, §6.2.

### D-SD-007 · 2026-10-08 · SD · Stop vs the 6s undo and Windows Hello
- A stop that lands inside an approve's 6s undo window cancels the pending approve. It's treated as undone and logged as stopped. A deny stays a deny.
- A stop during Windows Hello wins. A Hello result that arrives later is discarded.
Source: SD message to UX, 2026-10-08 20:36 ET (relayed by the parent agent).
Affects: MECHANICS-SPEC §3, §6; TODAY-FLOWS §14.
Restated by Studio Director, 2026-10-08 20:41 ET: a stop cancels a pending approve; the 6s undo stays on the card (D-CD-001, D-SDIR-003); only a steer leaves the undo alone.

### D-SD-008 · 2026-10-08 · SD · Manual move of a budget-queued task to a local model
Arriq may move a budget-queued task to a local model by hand. It needs no Hello. It's logged as an event, keeps its tier, and every approval the task requires still applies. It's shown in the app. Only Arriq's explicit action in the desktop app, never automatic.
_Wording corrected by SD 2026-10-08 20:40 ET._
Source: SD message to UX, 2026-10-08 20:36 ET (relayed by the parent agent).
Affects: MECHANICS-SPEC §1, §5; TODAY-FLOWS §4.3.

### D-SD-009 · 2026-10-08 · SD · Process kill on stop
On stop, the daemon kills the whole process tree (a Job Object on Windows, the process group on Linux): TERM, then KILL after 2s. Partial output is discarded. Nothing produced after the stop reaches a card or the event log as a result.
_Wording corrected by SD 2026-10-08 20:40 ET._
Source: SD message to UX, 2026-10-08 20:36 ET (relayed by the parent agent).
Affects: MECHANICS-SPEC §3.

### D-SD-010 · 2026-10-08 · SD · Stop-cancel authority and stop origins
A stop-cancel needs no approval-key signature and no Hello, because a stop must never be blockable and cancelling is the safe direction. The daemon records a `stopped` event on the signed event chain, citing the stop's origin. Stops are accepted only from authenticated trusted surfaces: desktop app IPC, plus the tailnet phone trigger once SD has reviewed it.
Source: SD message to UX, 2026-10-08 20:40 ET (relayed by the parent agent).
Affects: MECHANICS-SPEC §3, §9 (SD-M7 closed).

### D-SD-011 · 2026-10-08 · SD · No Skip on the handover
There is no Skip on the handover. Closing or dismissing it leaves the task paused in "needs you". Only an explicit task stop drops it.
Source: SD message to UX, 2026-10-08 20:40 ET (relayed by the parent agent).
Affects: TODAY-FLOWS §7.3, §7.5, §12 (SD-10 closed).

## Review rules

Published by Studio Director (plan v2, §Team and review rules). They apply from the next PR.

1. **One joint review per build.** All reviewers grade the same contact sheet at the same time and produce one combined findings list.
2. **One settler per domain.** CD settles design conflicts and SD settles security before any code changes. Then there is one fix pass.
3. **Passed stays passed.** A reviewer doesn't re-grade something that already passed.
4. **Two-round cap.** After 2 fix rounds, the remaining nits become tickets and the PR merges if CI is green and SD signs off.
5. **Rulings are logged** in `DECISIONS.md`. Reversing one (like the 6-second undo) needs Arriq's okay.
6. **Results-only status.** One digest from Studio Director every weekday at 8:45 AM ET, plus immediate pings only for blockers. Mon–Wed Oct 12–14 it uses the Today lobe layout.
7. **Machine access.** Demo builds go to a drop folder on the Ubuntu server, private to the tailnet, not through the Grok Bot app.
8. **Scoped bots don't review UI.** Homelab Ops, Windows Engineer and QA Lead report results; they never grade design.

## Studio Director (process and scope)

### D-SDIR-001 · 2026-10-08 · Studio Director · The phone fallback counts as manual intervention for G3
A morning where the phone fallback was used doesn't count as a hands-off morning in the G3 run.

### D-SDIR-002 · 2026-10-08 · Studio Director · #24's High fixes don't count toward the two-round cap
A fix for #24's High security findings is outside rule 4's two fix rounds.

### D-SDIR-003 · 2026-10-08 · Studio Director · Undo is 6 seconds, on the card
The approval undo window is 6s and lives on the glass card, not on the filed receipt (D-CD-001). Spec: LOOK §8.3 :125–128 and INTERACTIONS S5 :70–73. Reversing it needs Arriq's okay (rule 5).
