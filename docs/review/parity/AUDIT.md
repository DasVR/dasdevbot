# Mock parity audit

Ranked gap list for the dasdevbot desktop app against the locked HTML mocks. The mocks are the source of truth. This run records gaps. It does not change the app.

Structured records live in [`gaps.json`](./gaps.json). Side-by-side stills are in [`stills/`](./stills/). The card-rise clip is [`clips/card-rise.mp4`](./clips/card-rise.mp4).

## Heads audited

| Head | SHA | Branch |
| --- | --- | --- |
| main | `d68f331` | `main` |
| #21 | `ed88596` | `cursor/ink-press-fill-783d` |
| #22 | `e3171e4` | `cursor/thread-prototype-port-2b74` |
| #24 | `931ee96` | `cursor/desktop-shell-morph-0ea2` |
| #26 | `41254ae` | `cursor/approval-queue-d2b4` |
| #27 | `f201368` | `cursor/first-run-flow-611c` |

`tokens.css` in the look pack hashes to `b78f108c1c09604ba70ca1dc7f00620d12acad0e6826eb1bc5f827e004ef0427`. App theme tokens match that file except `--ink-press: #1E1B18`, which exists only on #21 (G05).

## Counts

| Severity | Count | Meaning |
| --- | --- | --- |
| P0 | 2 | Wrong behavior, broken layout, or a trust failure |
| P1 | 13 | Visible look or feel mismatch in a primary flow |
| P2 | 10 | Minor pixels |
| Total | 25 | |

A gap is counted on every head that still has it.

| Head | P0 | P1 | P2 |
| --- | --- | --- | --- |
| main | 2 | 6 | 3 |
| #21 | 2 | 3 | 2 |
| #22 | 1 | 5 | 3 |
| #24 | 2 | 3 | 3 |
| #26 | 0 | 5 | 3 |
| #27 | 2 | 2 | 1 |

#26 is the only head with both the flat destructive row and a Hello seam. That seam is still the wrong chrome (G14), so the external-approve path is not closed.

## Method

1. Mapped each mock and each locked prototype to the app surface that renders it.
2. Served the look pack and each UI head. Chrome, `deviceScaleFactor` 2, timezone `America/New_York`, Figtree 400/620 and JetBrains Mono 400 loaded (`document.fonts.check`). Viewports 1280×800 and 1440×900.
3. For each matched element, compared 27 computed properties: width, height, padding, margin, radius, font family, size, weight, line-height, letter-spacing, color, background, border width/style/color, box-shadow, backdrop-filter, opacity, z-index, transform, and stroke width (non-scaling). Page x/y is excluded when the shells differ. Content height is excluded for the card, because copy length changes it.
4. Lifted easing, duration, delay, spring, keyframe, hold, undo, streaming, and shell-morph values from the mock source and compared them to the app source.
5. Stepped the mock clock with `window.__cap.step(16.667)` and the shell clock with `window.__shellCap.step`. Sampled hover, press, and reduced motion (`?rm` on the mock, `prefers-reduced-motion` on the app).

### Measurement limits

- Playwright's clock repeated CSS animation samples on the app card rise (the same opacity for several 16.667ms steps; frame 44 was still about 0.39). Rise duration is taken from source on both sides and from the mock frames. It is 520ms `var(--ease-out)` from `translateY(12px) scale(0.98)`. That match is not filed as a gap.
- `clips/card-rise.mp4` is 18 frames at 12fps (every other virtual frame), 960×360, about 50KB. It shows the rise. It is not the 60fps proof.
- Focus outline (2px accent in source) was not in the sampled property set. Disabled opacity 0.45 is in `ApprovalCard` button CSS and was not printed in the state summary.
- Some mock beat probes (`5a`, `6a`, `1c`, `7c` rules) fired before the chapter DOM crossfaded. Those frames are not scored. The GitHub step of `7c` and the evening greeting copy on #27 were sampled after the DOM settled.
- The companion frame of `4a` was captured mid-tween (window width 437px) because the seek stopped as soon as width dropped under 500. Settled companion geometry is the app `snap()` against `MOCK_FORMS`, which matches: window 400×790 at (1000, 72) radius 14; composer 376×52 at (1012, 798) radius 26.
- An early queue probe that read "Review 0" was a snapshot race. The re-probe rendered "Review 3" in oldest-first order. That race is not a gap.

## Surface map

| Mock | App surface | Where it lives |
| --- | --- | --- |
| `mocks/1a-voice-talk.html` | Push-to-talk mic lane | No head |
| `mocks/1b-voice-approve.html` | Voice approve | DEVIATION. Not built: security rule C6, voice never approves (`DASDEVBOT-LOOK.md:325`) |
| `mocks/1c-voice-destructive.html` | Flat destructive row | #22 and #26. Clay card on main, #21, #24, #27 |
| `mocks/2a-approval-hold.html`, `approval-card.html` | Waiting card, hold, receipt | main, #21, #22, #26 |
| `mocks/3a-modes.html` | Here / Focus / Away | Radios on #24. Return sheet missing |
| `mocks/4a-shell-morph.html` | Full, companion, pill | #24 |
| `mocks/5a-notify-review.html` | OS notification | In-app toast only, on #26 |
| `mocks/6a-live.html` | Island, menu flyout, tray | Island and menu flyout: DEVIATION, Phase 1 is Windows only (`DASDEVBOT-LOOK.md:328`). Tray numeral on #26, not frame-matched |
| `mocks/7a-characters.html` | Mascot study | Curl is the mascot. Other characters are not a build target |
| `mocks/7b-greeting.html` | Daily greeting | #27 |
| `mocks/7c-first-run.html` | First-run steps | #27 |
| `thread.html` + `thread/` | Chat thread, composer, tools, handoff | #22 |
| `interactions.html` | Roster drag (the only spring) | No head. Shell morph uses ease-out |

## Parity scores

Matched properties over the 27-property set. The pen-stroke miss (mock computed 2.33px; the app is 1.75px) is a DEVIATION under the one-pen rule (`DASDEVBOT-LOOK.md:58`) and still counts as a raw miss.

| Surface | Viewport | Matched | Total | Rate |
| --- | --- | --- | --- | --- |
| Approval card chrome, main vs `2a` | 1280×800 | 123 | 133 | 92.5% |
| Approval card chrome, #21 vs `2a` | 1280×800 | 123 | 133 | 92.5% |
| Approval card chrome, #26 vs `2a` | 1280×800 | 123 | 133 | 92.5% |
| Thread composer, #22 `shot=composer` vs `thread.html` | 1280×800 | 92 | 108 | 85.2% |
| Shell full window, #24 vs `4a` | 1440×900 | 100 | 108 | 92.6% |
| Shell window box alone, #24 vs `4a` | 1440×900 | 30 | 30 | 100% |
| Shell full window, #24 vs `4a` | 1280×800 | 42 | 54 | 77.8% |
| First-run GitHub step, #27 vs `7c` | 1280×800 | 101 | 108 | 93.5% |

The #21 resting sample matches main because `--ink-press` only shows on press. The lit composer shot on #22 matches the glass mock. The approval shot's composer is paper until hover, and that is G08.

## Motion

Shared motion tokens match the look pack on every head: `--dur-fast` 140, `--dur-base` 240, `--dur-soft` 360, `--dur-draw` 300, `--dur-stage` 520, `--dur-hold` 600, `--dur-hold-destructive` 1200, `--dur-retract` 160, `--dur-stamp` 400, `--dur-trace-read` 2100, `--dur-trace-write` 1700. Easings match: out `cubic-bezier(.22,1,.36,1)`, inout `(.65,0,.35,1)`, press `(.3,0,.5,1)`, exit `(.4,0,1,1)`, draw `(.5,.05,.2,1)`.

Mock card-rise frames reach opacity 1 and `transform: none` by about frame 44 (520–600ms) from `matrix(0.98, 0, 0, 0.98, 0, 12)`. App source is the same 520ms ease-out. The 600ms hold, the 1200ms destructive hold, and the 6s undo are the values in source. The shell morph is ease-out, which matches `4a`. `--spring-soft` is specified for a drag of 8px or more. No head implements that drag.

Hover, normal motion: mock and app both lift about 1px (`translateY` near −0.7px). Press sampled at 30ms is mid-transition (scale about 0.994) toward 0.97 × 0.955. Source durations match.

Reduced motion: the app rise is a 160ms fade with no transform, and the press transform is `none`. `2a` under `?rm` still hover-translates and press-scales. The app is a DEVIATION from those frames under the reduced-motion rule, no transforms anywhere (`DASDEVBOT-LOOK.md:320`). #21's reduced-motion press background measured `rgb(30, 27, 24)` (`#1E1B18`). The other heads stay `rgb(43, 39, 35)`.

## Unimplemented

| Mock | Surface |
| --- | --- |
| `mocks/1a-voice-talk.html` | Push-to-talk mic lane. Phase 1 also lists voice as out of scope. |
| `mocks/5a-notify-review.html` | OS notification whose only action is Review. #26 has an in-app toast with Review. |
| `mocks/3a-modes.html` | Welcome-back review sheet when returning from Away. #24 draws the three modes and does not run the return sheet. |
| `mocks/6a-live.html` | Dynamic Island and the macOS menu flyout: DEVIATION, Phase 1 is Windows only, so the island and the menu bar are out of scope (`DASDEVBOT-LOOK.md:328`). The tray numeral on #26 was not frame-matched. |
| `interactions.html` | Roster drag, the only spring in the spec. |

## Classified by rule

These are not gaps. Each row is PARITY or DEVIATION and cites the rule it follows.

| Class | Rule | Cite |
| --- | --- | --- |
| DEVIATION | Security rule C6. Voice never approves. `1b` is not a build target. The reply is "Approve on screen." | `DASDEVBOT-LOOK.md:325` |
| DEVIATION | Security rule C1. Destructive is denied, never asked. `1c` still plays a 1200ms voice hold, which the rule removes. The app shows the flat ink row (#22, #26, and #24 from 38ded72). | `DASDEVBOT-LOOK.md:326` |
| DEVIATION | Reduced motion has no transforms. `2a` under `?rm` still translates on hover and scales the button to 0.97, 0.955. The app rise is a 160ms fade and the press transform is none. | `DASDEVBOT-LOOK.md:320` |
| PARITY | The receipt is 60px. The mock row is `min-height: 58px`. The filed article on main measured 60px tall, which meets that min-height. | `mocks/2a-approval-hold.html:193` |
| DEVIATION | The stamp is `HH:MM:SS EDT`. The filed mock stamp read "4:31:08 PM". The app stamp read "11:14:07 EDT". | `DASDEVBOT-LOOK.md:127` |
| DEVIATION | Pens are 1.75. The mock approve stroke computed at 2.33px. The app pen is 1.75px. Line icons on the app are 1.5. | `DASDEVBOT-LOOK.md:58` |
| DEVIATION | The mascot is Curl. `7a` is a character study. The other characters are not a build target. | `SCREENS.md:197` |
| DEVIATION | Windows only. The island and the menu bar in `6a` are out. The tray stays. | `SCREENS.md:172` |
| PARITY | During the 6s undo the card stays glass. A glass card in that window follows the ruling. | `DASDEVBOT-LOOK.md:321` |
| PARITY | The queue is oldest-first and does not auto-advance. #26 rendered Reviewer 11:14, then the expiring row in place ("expires 1:30"), then Builder 11:52. Header count was 3. | `SCREENS.md:160` |
| DEVIATION | Builder's sub drops the video's `lease bld_02` segment: lease identifiers never reach the DOM (DE ruling 2, CD ruling b, UI Designer #24 item 7). The line keeps the video's two-line shape, "2m14s · 208 /" then "8000 tok" (`lease-dom-smoke`). | `apps/desktop/src/lib/shell/roster.ts:1-5` |
| DEVIATION | Drawn Windows caption buttons at native metrics, OS chrome, Windows-only Phase 1 (CD ruling a). The mock's traffic-light dots are macOS chrome; the app draws 46px Win11 caption buttons with Snap Layouts on maximize. | `DASDEVBOT-LOOK.md:328` |

Hidden cards cannot be approved: the seen lock is an `IntersectionObserver` plus `seenArmed` (`SEEN_LOCK_MS` 800) before approve. This audit did not run a separate offscreen approve attempt.

## Needs a CD ruling

| Id | Topic | Mock | Spec | Standing |
| --- | --- | --- | --- | --- |
| CD1 | Approve resting fill | `mocks/2a-approval-hold.html:283` clips `.btn.approve > .ink` with `inset(0px 100% 0px 0px)`. Resting background measured `rgb(251, 249, 245)`. | `DASDEVBOT-LOOK.md:116` says the Approve draft is filled ink. | The founder rule says the mock wins, so G03 is filed. The written spec disagrees. |
| CD2 | Token streaming | `thread.html` streams the reply token by token. #22 follows that prototype. | `INTERACTIONS.md:143` says teammate messages arrive whole, with no typewriter. | Not scored until CD picks one. |

## Ranked gaps

Stills, left mock and right app unless noted: [`stills/card-waiting.png`](./stills/card-waiting.png), [`stills/card-thread.png`](./stills/card-thread.png), [`stills/destructive.png`](./stills/destructive.png) (main clay card beside the #22 flat row), [`stills/shell-full.png`](./stills/shell-full.png), [`stills/shell-pill.png`](./stills/shell-pill.png), [`stills/queue.png`](./stills/queue.png), [`stills/first-run.png`](./stills/first-run.png).

### P0

#### G01 — destructive approval

- **Surface:** destructive approval. **Owning:** main, also #21, #24, #27.
- **Mock:** `DASDEVBOT-LOOK.md:326` — flat ink row, dash, no dot: "Destructive actions are off in this build."
- **App:** `apps/desktop/src/lib/api.ts:133` — `effectWhy(destructive)` returns "overwrites history. Can't be undone once it runs" and the card renders it. `holdDurationMs` uses 1200ms at `api.ts:194`.
- **Delta:** main shows a clay Destructive card. #22 (`model.ts:35`) and #26 (`App.svelte:787`) show the flat row and no glass card.
- **Fix:** On every head, render the flat dash row and never mount a destructive card or a 1200ms hold.

#### G02 — external approve

- **Surface:** external approve. **Owning:** main, also #21, #22, #24, #27.
- **Mock:** `DASDEVBOT-LOOK.md:327` — seen lock, then the 600ms hold, then Windows Hello, then the 6s undo.
- **App:** `apps/desktop/src/lib/ApprovalCard.svelte:796` — the card commits from the hold. There is no Hello step.
- **Delta:** An external approve can finish without Hello. #26 opens a confirm step (G14). The other heads do not.
- **Fix:** After the 600ms hold, call Windows Hello before the undo window. Cancelling returns the glass card with no error.

### P1

#### G03 — Approve button

- **Surface:** Approve button. **Owning:** main, also #21, #22, #26.
- **Mock:** `mocks/2a-approval-hold.html:283` — `clip-path: inset(0px 100% 0px 0px)` on `.btn.approve > .ink`. Resting background measured `rgb(251, 249, 245)`.
- **App:** `apps/desktop/src/lib/ApprovalCard.svelte:1225` — `background: var(--ink-1)`. Resting background measured `rgb(43, 39, 35)`, text `rgb(251, 249, 245)`.
- **Delta:** The mock's ink arrives across the hold. The app is solid ink before the hold starts. See CD1.
- **Fix:** Start the approve control as the paper line button and reveal the ink clip over `--dur-hold`.

#### G04 — Approve button label

- **Surface:** Approve button label. **Owning:** main, also #22, #24, #26, #27.
- **Mock:** `DASDEVBOT-LOOK.md:125` — the `.face-done` label is deleted. The check is the confirmation.
- **App:** `apps/desktop/src/lib/ApprovalCard.svelte:783` — `span.face-done` reads "Approved".
- **Delta:** The label is still in the button. #21 removed it.
- **Fix:** Delete `.face-done` on every head that still has it.

#### G05 — ink press

- **Surface:** ink press. **Owning:** main, also #22, #24, #26, #27.
- **Mock:** `tokens.css:19` — `--ink-press: #1E1B18`.
- **App:** `apps/desktop/src/lib/styles/tokens.css:18` — the token is absent. Computed `--ink-press` is empty. Reduced-motion press stays `rgb(43, 39, 35)`.
- **Delta:** Reduced-motion press does not darken. #21 defines the token and the reduced-motion press measured `rgb(30, 27, 24)`.
- **Fix:** Land `--ink-press` from #21 and use it for every ink-button press, instant under reduced motion.

#### G06 — card quiet line

- **Surface:** card quiet line. **Owning:** main, also #21, #22, #26.
- **Mock:** `DASDEVBOT-LOOK.md:338` — "Hold, then confirm with Windows Hello. Nothing posts until the 6s undo closes."
- **App:** `apps/desktop/src/lib/ApprovalCard.svelte:796` — "Records your decision. Nothing is posted in this demo."
- **Delta:** The armed mock quiet line is the hold hint ("hold approve · hold deny"). The app keeps the demo sentence.
- **Fix:** Replace the demo sentence with the Phase 1 small print, and show the key hint once the seen lock arms.

#### G07 — shell composer

- **Surface:** shell composer. **Owning:** #24.
- **Mock:** `mocks/4a-shell-morph.html` — full and companion composer measured `rgba(248, 243, 234, 0.78)` with `blur(24px) saturate(1.5)`.
- **App:** `apps/desktop/src/lib/shell/motion.ts:318` — mat paper opacity is 1 unless the form is pill. Mat glass opacity is 0. Companion `.mat.glass` measured opacity 0.
- **Delta:** Glass is only the pill. Full and companion composers are paper.
- **Fix:** Keep the composer glass in full and companion. Paper is the settled receipt material.

#### G08 — thread composer

- **Surface:** thread composer. **Owning:** #22.
- **Mock:** `thread.html` — resting composer is 720×60, radius 32px, glass fill and `blur(24px)`. Measured on `?capture` before play.
- **App:** `apps/desktop/src/lib/thread/ComposerBar.svelte:18` — glass is `lit || hovered || focused`. Resting background is `var(--paper-raised)` at line 92.
- **Delta:** The approval shot's composer measured paper and blur none. The lit shot matches the mock.
- **Fix:** Make the composer glass at rest. Hover only moves the sheen.

#### G09 — composer field

- **Surface:** composer field. **Owning:** #22.
- **Mock:** `thread.html` — textarea font-family Figtree, 14px / 21px.
- **App:** `apps/desktop/src/app.css:88` — `button, input { font-family: inherit }`. The textarea is not reset, and `ComposerBar.svelte:268` does not set a family. Computed family was monospace.
- **Delta:** The message field is the UA monospace face.
- **Fix:** Set the textarea to `var(--font-ui)`, 14px / 21px.

#### G10 — filed receipt

- **Surface:** filed receipt. **Owning:** main.
- **Mock:** `mocks/2a-approval-hold.html:193` — width 520px, radius 14px, paper. Measured 520×58.
- **App:** `apps/desktop/src/App.svelte:780` — `width: min(520px, 100%)` applies only to `.slot.over`. The filed article measured 674×60, radius 14px, paper.
- **Delta:** The filed row stretches to the stream column. Height 60 matches the CD ruling. #22's receipt measured 520×60.
- **Fix:** Keep the filed receipt at 520px in the thread slot.

#### G11 — waiting card placement

- **Surface:** waiting card placement. **Owning:** main.
- **Mock:** `mocks/2a-approval-hold.html:117` — the card is a 520px glass layer in the thread. Measured at x=308, y=156.
- **App:** `apps/desktop/src/App.svelte:766` — `.slot.over` is `position: fixed; bottom: 16px`.
- **Delta:** The waiting card floats on the bottom edge. In the mock it sits in the thread under its tool row.
- **Fix:** Use the #22 thread slot. Drop the fixed overlay.

#### G12 — shell at 1280×800

- **Surface:** shell at 1280×800. **Owning:** #24.
- **Mock:** `mocks/4a-shell-morph.html` — window stays 1360×828, radius 14px, at x=40, y=52.
- **App:** `apps/desktop/src/lib/shell/geometry.ts:110` — `stageScale` is `min(width/1440, height/900)`. At 1280×800 the window measured 1208.88×736, radius 12.44px.
- **Delta:** At 1440×900 the window matched 30/30 properties. At 1280 the app scales and the mock keeps 1440-stage pixels.
- **Fix:** At 1440 the stage is already right. The 1280 bar is the mock's pixels until CD says otherwise.

#### G13 — first-run primary

- **Surface:** first-run primary. **Owning:** #27.
- **Mock:** `mocks/7c-first-run.html` — Connect GitHub measured with a 1px ink border and the ink contact shadow.
- **App:** `apps/desktop/src/lib/FirstRun.svelte:522` — `button.primary` measured border 0 and box-shadow none. Width 136.56 vs 138.56.
- **Delta:** The ink button is a flat fill. Title, "2 of 4", and the 48px slot matched.
- **Fix:** Use the same ink button shadow and 1.5px edge as Approve.

#### G14 — Windows Hello

- **Surface:** Windows Hello. **Owning:** #26.
- **Mock:** `DASDEVBOT-LOOK.md:327` — Hello is OS chrome and is never restyled.
- **App:** `apps/desktop/src/lib/ApprovalCard.svelte:862` — in-app Cancel and Confirm buttons. `devWindowsHello.confirm` resolves true.
- **Delta:** The line "Confirm with Windows Hello" is right. The auth control is a styled pair of text buttons.
- **Fix:** Call the OS prompt from the seam. Do not draw Confirm.

#### G15 — card padding

- **Surface:** card padding. **Owning:** main, also #21, #22, #26.
- **Mock:** `mocks/2a-approval-hold.html:117` — `padding: var(--s-2) var(--s-2) 16px`. Quiet margin-top 10px at line 176.
- **App:** `apps/desktop/src/lib/ApprovalCard.svelte:875` — `padding: var(--s-2) var(--s-2) var(--s-5)`, so 8px 8px 20px. Quiet margin measured 12px.
- **Delta:** Bottom padding is 4px heavy. The quiet line sits 2px lower.
- **Fix:** Use 16px bottom padding and 10px quiet margin.

### P2

#### G16 — card shadow

- **Surface:** card shadow. **Owning:** main.
- **Mock:** `mocks/2a-approval-hold.html:117` — element shadow is glass-edge. The float shadow is a separate `.lift` layer.
- **App:** `apps/desktop/src/lib/ApprovalCard.svelte:874` — `box-shadow: var(--glass-edge), var(--shadow-float)` on the card.
- **Delta:** The float shadow is painted on the glass box, so the edge reads heavier.
- **Fix:** Keep glass-edge on the card and put `--shadow-float` on the lift layer.

#### G17 — button contact shadow

- **Surface:** button contact shadow. **Owning:** main.
- **Mock:** `mocks/2a-approval-hold.html:140` — deny shadow uses `rgb(var(--shade) / .10)` and `/ .18`.
- **App:** `apps/desktop/src/lib/ApprovalCard.svelte:1244` — measured shadow alphas 0.06 and 0.16 (`--shadow-puff`) instead of 0.10 and 0.18.
- **Delta:** The paper button's contact shadow is lighter than the mock.
- **Fix:** Match the mock's two-layer button shadow.

#### G18 — send disc

- **Surface:** send disc. **Owning:** #22.
- **Mock:** `thread.html` — send measured radius 50%, font-size 14px, stroke 1.7px.
- **App:** `apps/desktop/src/lib/thread/ComposerBar.svelte:76` — measured radius 999px, font-size 13.333px, stroke 1.6px.
- **Delta:** The disc is a pixel smaller in type and stroke. 999px and 50% are the same pill.
- **Fix:** Set the icon to 16px and the stroke to the 1.5 line-icon weight. The mock's 1.7px is a raster of that stroke.

#### G19 — composer chip

- **Surface:** composer chip. **Owning:** #22.
- **Mock:** `thread.html` — chip line-height 12.5px, background `rgba(237, 231, 221, 0.94)`.
- **App:** `apps/desktop/src/lib/thread/ComposerBar.svelte:256` — measured line-height 21px and background `rgb(237, 231, 221)`.
- **Delta:** The chip is a solid paper pill with body line-height.
- **Fix:** Use the mock's 12.5px line and the 0.94 paper tint.

#### G20 — queue meta

- **Surface:** queue meta. **Owning:** #26.
- **Mock:** `mocks/3a-modes.html` — sheet row measured "DasVR/NIL #212 · external · 11:14".
- **App:** `apps/desktop/src/lib/queue.ts:87` — `queueMeta` joins repo, ref, risk, and time. Rendered "DasVR/NIL · 212 · external · 11:14 AM".
- **Delta:** The hash before the PR number is missing. Order and the static expires label are right.
- **Fix:** Prefix a numeric ref with `#`.

#### G21 — daily greeting

- **Surface:** daily greeting. **Owning:** #27.
- **Mock:** `mocks/7b-greeting.html` — greeting block measured 302.34×57.
- **App:** `apps/desktop/src/App.svelte:239` — `[data-greeting]` measured 806×59. Copy was "Evening, Arriq." / "Nothing waiting · 0 done today."
- **Delta:** Type matches (Figtree 14/21, ink-1). The block is the full pane width.
- **Fix:** Size the greeting to the mock's line width and keep Curl in the 48px slot.

#### G22 — shell roster

- **Surface:** shell roster. **Owning:** #24.
- **Mock:** `mocks/4a-shell-morph.html` — roster background `rgba(237, 231, 221, 0.55)`, padding-bottom 0.
- **App:** `apps/desktop/src/lib/shell/Shell.svelte:363` — measured padding-bottom 16px and an oklab tint at the same alpha.
- **Delta:** The wash is the same color in a different serialization, plus 16px of extra bottom padding.
- **Fix:** Drop the extra padding. Keep the 0.55 sunken wash.

#### G23 — risk strip

- **Surface:** risk strip. **Owning:** main.
- **Mock:** `mocks/2a-approval-hold.html:130` — `.card > .risk { z-index: 1 }`. Strip measured 504×36, radius 20px, 12.5px / 620.
- **App:** `apps/desktop/src/lib/ApprovalCard.svelte:667` — z-index computed `auto`. Size, radius, type, and color matched.
- **Delta:** The strip can slide under the glass rim.
- **Fix:** Set z-index 1 on `.risk`.

#### G24 — first-run slot

- **Surface:** first-run slot. **Owning:** #27.
- **Mock:** `mocks/7c-first-run.html` — `.fr .slot` margin-left 28px. The slot is 48×48.
- **App:** `apps/desktop/src/lib/FirstRun.svelte:399` — slot measured 48×48, margin-left 0. On the helper step at 1440 the slot was x=400, y=120.
- **Delta:** The box is the right size. It sits flush where the mock insets 28px.
- **Fix:** Inset the 48px slot to the mock's x within the column, and keep that x on every step.

#### G25 — shell composer padding

- **Surface:** shell composer padding. **Owning:** #24.
- **Mock:** `mocks/4a-shell-morph.html` — composer padding-left and padding-right 10px.
- **App:** `apps/desktop/src/lib/shell/Shell.svelte:423` — the composer box padding measured 0. The mat fills the box edge to edge.
- **Delta:** The chip and field are flush with the glass edge. The mock insets them 10px.
- **Fix:** Pad the composer row 0 10px, matching the thread composer.
