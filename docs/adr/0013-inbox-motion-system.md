# 0013 One motion system for the menu-bar inbox

Status: Proposed, 2026-10-28.

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

## Decision

Every motion of the inbox comes from one coordinator, `InboxTransition`, in four steps per change. No view animates itself.

1. **Diff by stable id.** The content is one vertical stack of slots: header, banner, list rows or the placeholder, footer, with their rules. A slot's id is stable: a row's `rowId` (a superseding message keeps its predecessor's), a section's kind. The placeholder is the slot the stack shows when it has no rows, not a separate mechanism. Before and after are diffed by id into operations: insert, remove, move, resize, update (content changed in place).
2. **Lay out once.** One pure layout pass measures each slot once and gives the after-frames, the card height and the scroll document height. The before-frames are the presented ones. An inserted slot starts at height 0 at its place in the before-stack (the bottom of the slot above it as it shows now); a removed slot ends at height 0 at its place in the after-stack. Hidden slots sit at height 0 in the stack; nothing is parked elsewhere.
3. **One declared spec per operation.** All geometry (every slot's top and height, the card height, the window, the scroll origin) moves on the motion curve, 0.3 s, from one start time. Since before and after are each a gapless stack and every slot interpolates on the same curve and clock, the stack stays gapless at every frame. Content moves on its own declared spec, never on the geometry:

   | Operation | Place (slot) | Content |
   | --- | --- | --- |
   | insert | opens from 0 | fades in over 0.18 s, zooms 0.96 to 1 |
   | remove | closes to 0 | fades out over 0.12 s, zooms 1 to 0.96 |
   | move | slides, raised above the slots it passes | unchanged |
   | resize (expand, collapse, new text) | follows the new height | the body fades in or out as now |
   | update | unchanged | leaf text layers cross-fade, 0.3 s |
   | panel open, close | none | fade and zoom, 0.18 s / 0.12 s, on the motion curve |

   Under Reduce Motion the fades stay and the zooms go. The geometry keeps the curve, as the accordion does today.
4. **Commit in one transaction.** A change that arrives mid-motion adds to what is running instead of replacing it: each new animation is additive, from the old target minus the new to zero, under its own key. The motion already running keeps its speed, so a retarget never stalls. A completion block on the transaction ends leaving slots and shrinks the window; no timers.

The views keep what they own: a slot view owns its frame and is never scaled; an inner content layer is what fades and zooms, about its own full-size centre, clipped by the slot while the slot opens or closes. Stacking order follows the stack. The rows always live in the scroll view's document; the scroll view's height is the list slot's height, capped or not, so nothing changes parent. Cross-fades apply to leaf layers only (a label, an image), never to containers.

### Code

- `StackPlan` (pure, replaces `RowStack`): slots, the keyed diff, the before-stack and after-stack frames.
- `MotionSpec` (pure): the spec table above, from `Tokens`, with the Reduce Motion variant.
- `MotionTimeline` (pure): the animations of a plan as values (from, to, start, duration, curve, additive), and the value of any slot at any time.
- `InboxTransition`: turns a plan into layer animations in one transaction, and owns completion.
- `LayerMotion`'s free-standing fades, zooms, grows and transitions leave the inbox views; `InboxPanel.setCardHeight` takes the plan's card track.

## Consequences

- One rule explains every frame: the presented stack is the before-stack and the after-stack mixed by one curve, plus the content specs. Defects 1 to 9 cannot occur by construction; 10 and 11 by the spec table.
- The motion can be tested without a screen. `MotionTimeline` evaluates a plan at 120 Hz in a plain unit test: the last frame equals the target layout; consecutive visible slots never gap or overlap (pairs crossing in a move excepted, with the mover on top); no slot moves more in a frame than the curve's steepest step (0.111 of the distance) allows; bursts of inserts and removals end in the model's order. The tests join the `GentboxTests` source list. A debug probe checks that the presented layers match the timeline.
- Additive animations make a layer's presented value the sum of several animations; the timeline models the same sum, so the tests and the screen agree.
- A move slides through its neighbours. The alternative, closing at the old place and opening at the new, keeps the stack free of overlap but needs a copy of the content at both places; it is not taken.
