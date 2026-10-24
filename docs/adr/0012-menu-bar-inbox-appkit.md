# 0012 Menu-bar inbox in AppKit and Core Animation

Status: Accepted, 2026-10-24.

## Context

The menu-bar inbox must hold 120 Hz (ProMotion) with no dropped frames while it changes: a row expands into a card, an item arrives or leaves, the card follows a superseding message, the height follows the content. The user's rule for it: "Apply proper fix rather than workaround."

The first version was SwiftUI in an `NSHostingView`. Every content change cost one main-thread block of 17–28 ms in its first frame (two to three frames at 120 Hz) and 4–6 ms in each frame after it. Measured in Release, with `_printChanges`, Time Profiler and Instruments' Animation Hitches:

- **Not over-invalidation.** An expand re-evaluated only the root and the two rows it touched. A floor build with no card body and no hover menu still blocked 9–15 ms per first frame, and every frame of the motion re-ran SwiftUI layout on the main thread.
- **macOS 26 runs SwiftUI inside AppKit's controls.** Each `NSButton` and bezelled `NSTextField` builds its own SwiftUI graph: 3–8 ms to create and first draw. A plain AppKit process loads SwiftUI as soon as one exists.
- **The render server had its own cost.** A surface that spanned the whole panel column was clipped to the card by a mask layer (an offscreen pass each frame), and the shadow re-blurred an animated `shadowPath` each frame: 5–7 ms of GPU per frame.
- **AppKit's text-field focus does work nobody asked for.** The autofill heuristics took the reply field for a one-time-code field: on focus they walked every key view and built Safari's code-autofill window. The first keystroke set up the shared spell checker.

## Decision

The inbox content is AppKit views with manual frame layout, animated by Core Animation. SwiftUI stays only for the `App` entry point.

- **One snapshot per change, diffed onto views.** `InboxContentView` reads the model under `withObservationTracking` into a value snapshot, updates only the rows and sections whose inputs changed, lays out in one pass and commits every motion of the change (row positions, fades, the card's height) in one Core Animation transaction from the presented values (`LayerMotion`). Rows are reused by message id; a superseding message takes over its predecessor's row; filtered-out rows and unused sections are parked below the card, not removed.
- **Native controls built ahead.** The buttons and reply field of the cards a click or a hand-over can show next (the rows on screen and the next-up item) are created one per quiet run-loop turn (`IdleWork`: no input queued, no motion running), drawn at zero opacity, out of reach of clicks, keys and VoiceOver. Expanding only reveals them. Only the expanded card that stays takes keys: a card leaving, parked or hidden by a filter takes none, and a card handed over after an answer takes keys once its motion is over.
- **A card-sized surface.** The surface is the card itself, with its own rounded corners (`masksToBounds`, no mask layer); the shadow is a nine-slice image drawn once; the ring and the shadow live in layer-hosting views, because AppKit re-sorts a view's own sublayers by subview order.
- **No autofill on the reply field.** The app registers `NSAutoFillHeuristicsEnabled = false` in its own registration domain (in memory, this process only; Gentbox has no credential fields), and warms the field editor and the spell checker ahead of the first focus.
- **No redundant AppKit bookkeeping.** Labels are sized from their cell, not `fittingSize` (which solves a temporary Auto Layout engine per call); symbol images are looked up once per name and size; unchanged fills and frames are not set again.
- **Same text sizes in both states** (the user's decision): the expanded card uses the collapsed row's 13 pt semibold title and its context line; expanding adds content and actions and enlarges no text.

## Consequences

- Measured in Release at 120 Hz on a Mac14,6 (`apps/apple/README.md`, "Measuring it"): an expand benchmark of 16 expands went from 19–20 hitches to 0–1; the main-thread cost of an expand from 22–29 ms to 4–7 ms; GPU per frame from 5–7 ms to 1–2 ms. Per-transition pacing and the Animation Hitches summary for the final build are in the PR #5 body.
- Layout is code: every row, card body and chrome section computes its own frames. Text metrics come from AppKit's own labels, so the stills match the SwiftUI build to the device pixel.
- Accessibility is explicit: a collapsed row is one button element that reads urgency, title, context and wait state, with the value "Muted" when muted; an expanded card is a group of its parts.
- Popups and toasts (PR 4) start from the same parts (`Controls.swift`, `LayerMotion`, `IdleWork`).
- `NSAutoFillHeuristicsEnabled` is an AppKit defaults key, not public API. If a later macOS drops it, the reply field's first focus costs a few milliseconds again; nothing breaks.

## Alternatives

- **Accept one stall per change** (about two frames at 120 Hz). Rejected by the user.
- **Drop the per-row hover menu** (about 20 % less). Rejected as a workaround: the rest of the stall stays.
- **Equatable SwiftUI rows, scoped animation keys, stable identifiers.** Measured first; the floor build showed the cost is SwiftUI's per-frame layout and its first-frame setup, not invalidation.
- **AppKit list with SwiftUI card bodies.** The card body is where the native controls are, and on macOS 26 those carry SwiftUI graphs of their own; hosting a SwiftUI body per card adds a graph rather than removing one.
