# 0013 One motion system for the menu-bar inbox

Status: Accepted, 2026-10-28, amended after review.

## Context

ADR 0012 put the inbox on AppKit and Core Animation: one snapshot per change, one layout, one transaction. The motion inside that transaction grew piece by piece: each kind of change got its own call (`arrive`, `send`, `show`, `hide`, `grow`, `zoom`, `crossfade`, `setCardHeight`), each placing and timing its own layers. In the user's trial the lists spawned oddly, the empty-inbox placeholder moved strangely, and most motions looked off.

A headless probe recorded every presented frame at 120 Hz (rows, card bodies, sections, the card) through twenty scripted changes: populate, expand, add three, withdraw (newest, middle, expanded), supersede, change text, toggle a waiter, a burst of three changes 60 ms apart, clear, add to empty, and the same under Reduce Motion. Frames where a layer had no presentation yet were left out.

### What moves today

| Element | Trigger | Frame and timing owner | Curve, duration |
| --- | --- | --- | --- |
| Card height (surface, ring, shadow) | content height changes | `InboxPanel.setCardHeight` | motion curve, 0.3 s; window shrink by timer |
| Panel open and close | status item | `InboxPanel.spawn` / `despawn` | ease-out 0.18 s / ease-in 0.12 s |
| Row place (top, height) | any change | `layOutRows` through `RowStack`, captured before the change | motion curve, 0.3 s |
| Row arrival | insert, bring back | `arrive`: height from 0, fade, zoom 0.96 to 1 | motion curve, 0.3 s |
| Row departure | withdraw, filter | `send`: height to 0, fade, zoom 1 to 0.96, finish by timer | motion curve, 0.3 s |
| Card open and close | expand | `InboxRowView.showBody` / `hideBody` | body fade in halves; quick fade out 0.08 s |
| Header change | content change | `InboxRowView.apply`: `CATransition` over five layers | motion curve, 0.3 s |
| Body change | supersede, edit | `CardBodyView.show`: `CATransition` over the summary and the code box | motion curve, 0.3 s |
| Buttons | waiter comes or goes | `CardBodyView.showBar`: fade and zoom per control | motion curve, 0.3 s |
| Sections (placeholder, banner, list, footer, rules) | section set changes | `show` / `hide` fades; parked below the card when unused | motion curve, 0.3 s; hide finishes by timer |
| Picking | picking starts or ends, a pick | `InboxRowView.showCheckbox` (`InboxRowView.swift:176`): checkbox, icon and urgent badge fade; the check symbol and the tint change at once | motion curve, 0.3 s; the pick none |
| Hover | the pointer enters or leaves a row | `InboxRowView.showHover` (`:356`): fill, More button and waiting mark set at once; the first hover makes the More button and lays the row out again | none |
| Scroll | content passes the cap | `swapScrolling` reparents the list; scroll to the open card by timer | none; then a second motion |

### What goes wrong

1. **Sections fly in from where they were parked.** `park` (`InboxContentView.swift:347`) moves an unused section below the card; `captureEverything` (`:224`) captures every section's position, parked ones too, so a section that shows again slides from its parked place. On clear, the placeholder rises from 647 to 51 pt behind the leaving rows, under Reduce Motion as well (610 to 51). The list does the same on populate and on the first add to an empty inbox (234 to 51).
2. **The placeholder and the list hand over through two mechanisms.** The list and the placeholder cross-fade as sections (`show` / `hide`, `:305`–`:333`) while the rows inside the list fade and close on their own: a row's opacity is the product of both, about 0.3 halfway, and on clear the rows slide up towards the top while the placeholder rises through them.
3. **New rows open at their final place while their neighbours move.** The capture runs before `updateRows` (`:438`) creates the row, so its position is never captured; `arrive` (`:522`) animates only its height and opacity. Rows above it travel, it does not: gaps of up to 143 pt (populate) and overlaps of 98 pt (add three) and 114 pt (populate under Reduce Motion). The card follows the true stack height, so content runs up to 215 pt past its bottom edge and is cut.
4. **The zoom scales about the wrong centre.** `LayerMotion.zoom` scales about the centre of the layer's final bounds, while the same layer's height opens from or closes to 0 (`grow`, `send`). The visible sliver drifts by up to 8 pt. The place and the content are one layer.
5. **A change during motion restarts it from rest.** Every change replaces each animation with a new one from the presented value (`LayerMotion.commit`, `LayerMotion.swift:30`); the curve starts at zero speed. In the burst a growing row's height went 17.3, 23.6, 23.8, 24.9 pt per frame (6 pt/frame to 0.2) and its top reversed at the third change: a visible stall.
6. **Layout runs more than once per change, and some heights are not part of the motion.** `sectionsHeight` (`:658`) and `layOut` (`:569`) both lay out every section; a row lays out in `apply` and again in `refreshLayout`. The list view's height and the open card's body height (97 to 129 pt on an edit) are set without motion and snap in one frame.
7. **Timers instead of the animation clock.** `asyncAfter(motionDuration)` ends a leaving row (`:514`), hides a section (`:327`), scrolls to the open card (`:218`) and shrinks the window (`InboxPanel.swift:106`). They drift from Core Animation's clock under the debug panel's slow motion and when a change retargets: a row is discarded while a newer change still moves it, and the scroll runs as a second motion after the first.
8. **The list changes parent mid-motion.** `swapScrolling` (`:671`) moves the list into or out of the scroll view when the content crosses the cap, and sets the opacities at once.
9. **Stacking order is insertion history.** New rows are added on top (`addSubview` in `updateRows`), so a row that moves or zooms passes over or under its neighbours by accident: when a decision loses its waiter, the open card slides down under three rows.
10. **Cross-fades over containers.** `CardBodyView.show` (`CardBodyView.swift:69`) runs a `CATransition` over the code box, a container with its own background and border; `InboxRowView.apply` (`InboxRowView.swift:122`) over five header layers. A transition dissolves a snapshot of the whole subtree: the box's border and fill double during the fade, and a resize mid-fade stretches the old snapshot.
11. **Two curves outside the tokens.** The panel opens on ease-out and closes on ease-in (`InboxPanel.swift:295`, `:310`).
12. **The clip view snaps.** `layOut` (`InboxContentView.swift:569`) sets the scroll view's final frame at once, and `NSScrollView.tile()` sizes the clip view to it in the same frame: on a shrink the rows are cut at the new height while the card is still closing over them, and a shorter document can clamp the scroll offset in one frame.
13. **Picking and hover move outside any spec.** `showCheckbox` fades the checkbox, the icon and the urgent badge on a free-standing call while the pick's check and tint change in one frame; hover changes the fill, the More button and the waiting mark in one frame, and the first hover makes the More button and lays the row out a second time, outside the change's one layout.

## Decision

Every motion of the inbox comes from one coordinator, `InboxTransition`, in four steps per change. No view animates itself.

1. **Diff by stable id, level by level.** The content is nested stacks of slots, each slot a stable id and a height, laid top to bottom without gaps:
   - the card: header, banner, list, placeholder, footer, with their rules; a section's id is its kind;
   - the list, which is the scroll view's document: the rows; a row's id is its `rowId`, which a superseding message keeps;
   - a row: its header line and its body's parts (summary, code box, action bar or reply field); a part's id is its kind.

   A slot's height is its child stack's height plus its padding, so a change inside a row (the code box growing from 97 to 129 pt on an edit, the action bar arriving with a waiter) is a resize of the row one level up, and the rows below it move with it. The placeholder is the card's slot shown while the list has no rows, not a separate mechanism. Before and after are diffed by id at every level into operations: insert, remove, move, resize, update (content changed in place).

   Each level's order is merged: the after-order, with every removed slot placed after its nearest predecessor in the before-order that is still placed, first if none is (the rule of `RowStack.order`, `RowStack.swift:14`). A change that moves some slots and inserts or removes others is laid out in that one order: a moved or inserted slot at its after-place, a removed one where the rule puts it.
2. **Lay out once.** One pure layout pass measures each slot once and gives each level's after-frames, the card height, the document height and the scroll offset. The before-frames are the previous change's after-frames, its target stack, not what shows: what shows is the target plus the motion still running (step 4). An inserted slot starts at height 0 at its place in the before-stack, the bottom of the slot above it; a removed slot ends at height 0 at its place in the after-stack. Hidden slots sit at height 0 in the stack; nothing is parked elsewhere.

   A slot inserted while its neighbours still move, or returning after its removal completed, copies the running tracks of the slot above it, that slot's top's and height's, so it starts where that slot's bottom shows and rides the same leftover motion. Without them it would carry only this change's delta while the slot above still carries the older ones, and the two would open apart: defect 3 again, in a burst. A slot whose removal is still running when it comes back keeps its own tracks. The first slot of a stack has nothing above it: its top is its parent's, and it moves with its parent.
3. **One declared spec per operation.** Every slot's top and height and the card's height move on the motion curve, 0.3 s, from the change's one start time. Before and after are each a gapless stack and every track runs on the same curve and clock, so the stack stays gapless at every frame. The window and the scroll view's geometry do not ride the curve; they are sized around it (below). Content moves on its own declared spec, never on the geometry, on the motion curve, from the same start:

   | Operation | Place (slot) | Content |
   | --- | --- | --- |
   | insert | opens from 0 | fades in and zooms 0.96 to 1, both over 0.18 s |
   | remove | closes to 0 | fades out and zooms 1 to 0.96, both over 0.12 s |
   | move | slides, raised above the slots it passes | unchanged |
   | resize | follows the new height | unchanged; on expand the body fades in over the second half of the motion, on collapse it fades out over 0.08 s |
   | update | unchanged | leaf layers cross-fade, 0.3 s |
   | picking starts or ends | unchanged | the icon and the checkbox cross-fade as leaves, 0.3 s |
   | pick, hover | unchanged | shows in the frame of the event, as pointer feedback; the More button is built with the row |
   | panel open, close | none | fades and zooms together (from 0.96, to 0.98), 0.18 s / 0.12 s |

   Under Reduce Motion the fades stay and the zooms go. The geometry keeps the curve, as the accordion does today.
4. **Commit in one transaction.** A change that arrives mid-motion adds to what is running instead of replacing it: each track's new animation is additive, from the previous target minus the new one to zero, under a key never used before (`motion.<keyPath>.<serial>`). The motion already running keeps its speed, so a retarget never stalls; the presented value is the target plus every running track.

   Each change's transaction has one completion. It ends that change's leaving slots and drops its movers back to their place in the drawing order; the last change still running also shrinks the window and the scroll geometry. No timers. Completions are guarded:
   - A completion runs two 120 Hz frames before its change's motion has run (halfway through a change shorter than four frames). Its commit then lands while the render server still draws the motion; a commit after the motion has run waits on a display that has slowed to idle, and shows a frame late, a hitch. What a completion settles that early must not show before the motion has run: a mover lowered in the drawing order keeps its raise as a hold, an additive `zPosition` animation of no change that lasts to the end of its motion and goes when a later change raises it again; a leaving slot, faded out by then, goes with the fraction of a point it still had to close (0.3 pt in 300 at 0.3 s); the scroll drops each slide once it has run, by the clock it shares with the change, not when the change completes.
   - A leaving slot records the change that removes it, and a mover the change that moved it last. A completion acts on a slot only while the slot still records that completion's change: an id the filter brings back before its removal ends is not discarded, and a mover moved again stays raised.
   - Completions are idempotent. The same work runs when the views leave the window, on `settle()` and on a change without motion, which all end the motion at once; a completion that runs after them finds nothing left to do.
   - No key is reused, so no animation of one change is replaced by another's; a replaced animation would report that it did not finish, and its completion would run early.

**The window** cannot ride the curve: `NSWindow.setFrame` runs on the main thread, not in the render server. It stays still during the motion. At commit it grows, holding its top, to the tallest card the timeline reaches from now to the end of every running track: that covers the presented card and every live target, and also the case where tracks of opposite sign carry the card past both. It shrinks only at the last completion, to the card's target; the room it keeps below the card for the shadow holds the card's last fraction of a point for the two frames left. The card (surface, ring, shadow) rides the curve inside it, and the window's empty part below the card passes clicks through.

**Scrolling.** The rows always live in the scroll view's document; nothing changes parent. The list slot is a plain view that rides the curve like any slot and clips what it holds; the scroll view inside it does not move on the curve:
- *Clip and document.* `NSScrollView.tile()` sizes the clip view to the scroll view's frame at once, so the scroll view's frame and the document's height stay still during the motion, like the window: at commit they grow to the largest the list slot and the row stack reach over the motion, and they shrink at the last completion, cutting the list slot's last fraction of a point for the two frames left. The document is also kept at least as tall as the scroll view plus every offset the motion uses, so AppKit never clamps the offset mid-motion. What shows of the list is the list slot's height on the curve.
- *Offset.* The layout gives the target offset. At commit the clip view scrolls to it at once, which is what AppKit, the scroller and hit-testing read, and an additive translation of the document's layer, from the new offset minus the old to zero, keeps the rows showing where they were and carries them on the curve.
- *Anchor.* If the open card changed, the target offset is the least that shows it whole (its top, if it is taller than the view). Otherwise, when slots above the visible top change, the first row whose before-frame reaches below the visible top is the anchor: the target offset moves by the anchor's own change of top, so the anchor stays still on screen while rows open and close above it, out of view. At offset 0 nothing is above the view, and a row inserted at the top shows. The target is clamped to the document's after-height.
- *User scroll.* A scroll-wheel event or a scroller drag mid-motion ends the offset's motion: the clip view takes the offset as it shows (the target less the translation's value now), the translation is removed, and the user scrolls from what they see. The rows' own tracks keep running. The next change starts from the user's offset.

**The views** keep what they own: a slot view owns its frame and is never scaled; an inner content layer is what fades and zooms, about its own full-size centre, clipped by the slot while the slot opens or closes. Drawing order follows the merged order through `zPosition`; a mover is raised above the slots it passes and drops back once its move has run, unless a later change has moved it again. Hit-testing does not follow `zPosition`: AppKit tests subviews in their order, against model frames. The model frames are the after-stack, which never overlap and give a leaving slot no height, so no two slots compete for a click and subview order never decides one; a click mid-motion lands on the slot whose place it is going to be. Cross-fades apply to leaf layers only (a label, an image), never to containers: the code box keeps its fill and border while its text cross-fades.

### Code

- `StackPlan` (pure, replaces `RowStack`): the nested stacks, the keyed diff in the merged order, the before- and after-frames at each level, and which slots copy whose tracks.
- `MotionSpec` (pure): the spec table above, from `Tokens`, with the Reduce Motion variant.
- `MotionTimeline` (pure): every track as a value (key path, from, start, duration, curve, additive), the tracks still running carried from change to change, and the value of any slot, the card and the offset at any time; it also gives the window's and the scroll geometry's sizes for the motion.
- `InboxTransition`: turns a plan into layer animations in one transaction, and owns the completions, the window and the scroll geometry.
- `LayerMotion`'s free-standing fades, zooms, grows and transitions leave the inbox views; `InboxPanel.setCardHeight` takes the plan's card track, and the panel's open and close move to the motion curve.

## Consequences

- One rule explains every frame: at every level, the presented stack is the target stack plus the running tracks, all on one curve, plus the content specs. Defects 1 to 9 and 12 cannot occur by construction; 10, 11 and 13 by the spec table.
- The motion can be tested without a screen. `MotionTimeline` evaluates a plan at 120 Hz in a plain unit test:
  - the last frame equals the target layout;
  - consecutive visible slots never gap or overlap, pairs crossing in a move excepted, with the mover on top as drawn, its hold included; a slot that left leaves only what it still had to close;
  - no track moves more in a frame than its distance times the curve's steepest slope over one 120 Hz frame, 0.1124 (0.1103 for frames aligned with the start), asserted per track: a slot carrying several tracks may move by the sum of their bounds;
  - bursts of inserts and removals end in the model's order;
  - a change 0.1 s into a running motion that inserts a row below a moving one keeps the stack gapless.

  The tests join the `GentboxTests` source list. A debug probe checks the presented layers against the timeline within a tolerance: the timeline's Bézier solver matches Core Animation's only that closely.
- Additive animations make a layer's presented value the sum of several animations; the timeline models the same sum, so the tests and the screen agree. A retarget turns a motion round instead of stopping it, which can carry a slot briefly past its new target; the window and the scroll geometry are sized for that.
- During a shrinking motion the window and the scroll view are taller than what shows. The window's empty part is transparent and passes clicks through; the overlay scroller can be cut by the list slot until the last completion.
- A move slides through its neighbours. The alternative, closing at the old place and opening at the new, keeps the stack free of overlap but needs a copy of the content at both places; it is not taken.
