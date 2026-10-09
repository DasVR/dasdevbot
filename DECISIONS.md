# DECISIONS.md: dasdevbot rulings log

Every settled ruling goes here (plan v2, rule 5). Reversing a logged ruling needs Arriq's okay.
Settlers: CD for design (rule 2), SD for security. Format: id · date · settler · ruling · source.
IDs: `D-CD-###` Creative Director, `D-SD-###` Security Director, `D-SDIR-###` Studio Director (process and scope). Entries are never deleted or rewritten. To reverse one, add a new entry that names the old id and Arriq's okay.

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
For shipped screens, the mock videos outrank the HTML mocks, and the Security Phase 1 waivers (C1, C4, C6) outrank both. For the Today lobe, the CD spec is the reference, and the security waivers still outrank it. Detailed rulings stay in `/workspace/dasdevbot-look/` (DASDEVBOT-LOOK.md, INTERACTIONS.md, SCREENS.md, tokens.css).

### D-CD-004 · 2026-10-08 · CD · #24 passed at `a7662f9`; the remaining nits are tickets
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

## Security (settler: Security Director)

### D-SD-001 · 2026-10-08 · SD · #24's CSP and signed-IPC Highs are closed; #24 merged at `a7662f9`
SD confirmed the CSP High and the signed-IPC High CLOSED at `a7662f9`. Decisions are signed card-window Tauri IPC only, and HTTP decide and undo stay 403. #24 merged at exactly that head, in merge commit `7ce1f870313148afc09ea976f554cddcb556d9fd`.

### D-SD-002 · 2026-10-08 · SD · #36's H1/M2/M3 gate G1
H1 (the bearer reaches JS), M2 (unauthenticated calls to the local daemon) and M3 (no Job Object) gate G1, the Oct 16 non-mock Windows run, and each fix needs SD's review first. They did not gate the #24 merge. Arriq's Oct 1 demo waiver ends when a real provider runs on Windows. M1 (NSIS) must land before any installer goes to the demo drop folder. The undo-crash item and the other follow-ups stay as tickets. Tracked in #36.

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
