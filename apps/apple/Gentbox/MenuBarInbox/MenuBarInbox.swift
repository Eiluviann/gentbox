import GentboxCore
import SwiftUI

/// The menu-bar inbox (design: `mac-popover`): every open message on one
/// white surface with a rule between items, urgent first, then waiting, one
/// of them expanded into the full card. Topic and thread show only in each
/// row's breadcrumb; no section titles, no counters.
///
/// Three parts, as every Gentbox window: the header at the top, the content
/// at its natural height with no scrolling, and the foot at the bottom. The
/// panel is as tall as the three, so it grows and shrinks with the content:
/// every change animates with `Tokens.motion`, and the panel's height
/// follows on the same curve (`StatusItemController`). The one exception is
/// content taller than the screen: the panel stops at the screen
/// (`PanelLayout.isCapped`) and only then does the content scroll, between
/// the header and the foot.
struct MenuBarInbox: View {
	let app: AppModel
	let layout: PanelLayout
	/// Laid out at its natural height for the panel to learn its size.
	var isMeasuring = false

	var body: some View {
		let scrolls = layout.isCapped && !isMeasuring
		VStack(spacing: 0) {
			InboxHeader(app: app)
			Divider()
			switch app.phase {
			case .starting:
				InboxNotice(symbol: "bubble.left", title: "Starting Gentbox", caption: nil)
			case .failed(let reason):
				InboxNotice(symbol: "xmark", title: "Gentbox could not start", caption: reason)
			case .running(let inbox):
				InboxContent(inbox: inbox, scrolls: scrolls)
			}
		}
		.frame(width: Tokens.popoverWidth)
		.fixedSize(horizontal: false, vertical: !scrolls)
		// The final height, reported in the pass that starts its animation.
		.onGeometryChange(for: CGFloat.self) { $0.size.height } action: { height in
			if !isMeasuring, !scrolls {
				layout.naturalHeightChanged(to: height)
			}
		}
		#if DEBUG
		.overlay(alignment: .bottom) {
			if AnimationTraceMarker.isShown { AnimationTraceMarker() }
		}
		#endif
		.animation(Tokens.motion, value: LayoutKey(app: app))
		.frame(maxHeight: isMeasuring ? nil : .infinity, alignment: .top)
	}
}

/// How the inbox's content and its panel fit: the content's natural height
/// as it reports it, and whether the panel stopped at the screen's height,
/// so the content scrolls.
@MainActor
@Observable
final class PanelLayout {
	var isCapped = false
	@ObservationIgnored private(set) var naturalHeight: CGFloat = 0
	@ObservationIgnored var onNaturalHeightChange: ((CGFloat) -> Void)?

	func naturalHeightChanged(to height: CGFloat) {
		naturalHeight = height
		onNaturalHeightChange?(height)
	}
}

/// Everything that changes the inbox's height: a change to any of it
/// animates with `Tokens.motion`, which the panel's height follows.
private struct LayoutKey: Equatable {
	let phase: String
	let isRead: Bool
	let expanded: InboxModel.Expanded?
	let items: [String]
	let filter: String?
	let filterOpenCount: Int
	let hasSelection: Bool
	let failure: String?
	let muted: String?

	@MainActor init(app: AppModel) {
		let inbox = app.inbox
		switch app.phase {
		case .starting: phase = "starting"
		case .failed: phase = "failed"
		case .running: phase = "running"
		}
		isRead = inbox?.inbox != nil
		expanded = inbox?.expanded
		items = inbox?.visibleItems.map(\.id) ?? []
		filter = inbox?.filter?.threadId
		filterOpenCount = inbox?.filterOpenCount ?? 0
		hasSelection = !(inbox?.selection.isEmpty ?? true)
		failure = inbox?.failure
		muted = inbox?.inbox.flatMap { InboxWords.mutedSummary($0.muted) }
	}
}

/// "Inbox", and the settings menu (Open at Login, Quit).
private struct InboxHeader: View {
	let app: AppModel

	var body: some View {
		HStack(spacing: 8) {
			Text("Inbox")
				.font(.system(size: 17, weight: .semibold))
				.frame(maxWidth: .infinity, alignment: .leading)
			Menu {
				Toggle(
					"Open at Login",
					isOn: Binding(
						get: { app.loginItem.isEnabled },
						set: { app.loginItem.setEnabled($0) },
					),
				)
				if app.loginItem.needsApproval {
					Text("Allow Gentbox in System Settings › Login Items")
				}
				Divider()
				Button("Quit Gentbox") { NSApp.terminate(nil) }
					.keyboardShortcut("q")
			} label: {
				Image(systemName: "gearshape")
					.foregroundStyle(.secondary)
			}
			.menuStyle(.borderlessButton)
			.menuIndicator(.hidden)
			.fixedSize()
			.frame(width: 28, height: 28)
			.accessibilityLabel("Settings")
			.accessibilityIdentifier("inbox.settings")
		}
		.padding(EdgeInsets(top: 12, leading: 14, bottom: 10, trailing: 12))
		.onAppear { app.loginItem.refresh() }
	}
}

/// The list as an accordion (design: `mac-popover`): every open message in
/// order, one of them expanded in place into the full card (by default the
/// next-up one), the rest as two-line rows; then the foot. It scrolls only
/// when the panel stopped at the screen's height.
private struct InboxContent: View {
	let inbox: InboxModel
	let scrolls: Bool

	var body: some View {
		if let snapshot = inbox.inbox {
			if inbox.isEmpty {
				InboxNotice(symbol: "tray", title: "No agent needs you", caption: nil)
			} else if scrolls {
				ScrollViewReader { proxy in
					ScrollView {
						InboxList(inbox: inbox, moveTargets: snapshot.moveTargets)
					}
					.frame(maxHeight: .infinity)
					.task(id: inbox.expanded) {
						guard let id = inbox.expanded?.messageId,
							(try? await Task.sleep(for: .seconds(Tokens.motionDuration))) != nil
						else { return }
						// No anchor: the least scroll that shows it, none if it shows.
						proxy.scrollTo(id)
					}
				}
			} else {
				InboxList(inbox: inbox, moveTargets: snapshot.moveTargets)
			}
			InboxFooter(snapshot: snapshot, inbox: inbox)
		} else {
			InboxNotice(symbol: "tray", title: "Reading the inbox", caption: nil)
		}
	}
}

/// The filter banner and the items, a rule between them.
private struct InboxList: View {
	let inbox: InboxModel
	let moveTargets: [TopicChoice]

	var body: some View {
		VStack(alignment: .leading, spacing: 0) {
			if let filter = inbox.filter {
				FilterBanner(filter: filter, inbox: inbox)
				Divider()
			}
			ForEach(Array(inbox.visibleItems.enumerated()), id: \.element.id) { index, message in
				if index > 0 {
					// `.pp-line + .pp-line` (an.css:1140): a rule between items.
					Tokens.line.frame(height: 1)
				}
				InboxItem(
					message: message,
					isExpanded: inbox.isExpanded(message.id),
					moveTargets: moveTargets,
					inbox: inbox,
				)
				.id(message.id)
			}
		}
	}
}

/// After Close found open items: "2 items still open in Phase 3", counted
/// live as they close, and Show all to leave the filter. At 0 the banner
/// offers the plain Close.
private struct FilterBanner: View {
	let filter: InboxModel.GroupFilter
	let inbox: InboxModel

	var body: some View {
		let count = inbox.filterOpenCount
		HStack(spacing: 8) {
			Text(Self.words(count: count, title: filter.title))
				.font(.system(size: 11))
				.foregroundStyle(.secondary)
				.frame(maxWidth: .infinity, alignment: .leading)
				.accessibilityIdentifier("inbox.filter")
			if count == 0 {
				Button("Close thread") { Task { await inbox.closeFilteredThread() } }
					.primaryButton(size: .small)
					.accessibilityIdentifier("inbox.close-filtered")
			}
			Button("Show all") { inbox.clearFilter() }
				.buttonStyle(.plain)
				.font(.system(size: 12))
				.foregroundStyle(Color.accentColor)
				.accessibilityIdentifier("inbox.show-all")
		}
		.padding(EdgeInsets(top: 8, leading: Tokens.rowHorizontalPadding, bottom: 8, trailing: 12))
	}

	static func words(count: Int, title: String) -> String {
		switch count {
		case 0: "Nothing open in \(title)"
		case 1: "1 item still open in \(title)"
		default: "\(count) items still open in \(title)"
		}
	}
}

/// The foot: the picked rows' Dismiss, what is muted (with Unmute), and the
/// last failure.
private struct InboxFooter: View {
	let snapshot: Inbox
	let inbox: InboxModel

	var body: some View {
		let muted = InboxWords.mutedSummary(snapshot.muted)
		if muted != nil || inbox.failure != nil || !inbox.selection.isEmpty {
			Divider()
			VStack(alignment: .leading, spacing: 6) {
				if !inbox.selection.isEmpty {
					HStack(spacing: 8) {
						Text("\(inbox.selection.count) selected")
							.font(.system(size: 11))
							.foregroundStyle(.secondary)
							.frame(maxWidth: .infinity, alignment: .leading)
						Button("Cancel") { inbox.clearSelection() }
							.buttonStyle(DesignButtonStyle(size: .small))
						Button("Dismiss") { Task { await inbox.dismissSelected() } }
							.primaryButton(size: .small)
							.accessibilityIdentifier("inbox.dismiss-selected")
					}
				}
				if let failure = inbox.failure {
					Label(failure, systemImage: "xmark")
						.font(.system(size: 11))
						.foregroundStyle(Tokens.waiting)
						.accessibilityIdentifier("inbox.failure")
				}
				if let muted {
					HStack(spacing: 8) {
						Image(systemName: "bell.slash")
							.font(.system(size: 12))
							.foregroundStyle(.secondary)
						Text(muted)
							.font(.system(size: 11))
							.foregroundStyle(.secondary)
							.frame(maxWidth: .infinity, alignment: .leading)
							.accessibilityIdentifier("inbox.muted-summary")
						Menu("Unmute") {
							ForEach(snapshot.muted, id: \.id) { group in
								Button(group.title) {
									Task {
										switch group.kind {
										case .topic: await inbox.setTopicMuted(group.id, false)
										case .thread: await inbox.setThreadMuted(group.id, false)
										}
									}
								}
							}
						}
						.menuStyle(.borderlessButton)
						.menuIndicator(.hidden)
						.fixedSize()
						.font(.system(size: 12))
						.foregroundStyle(Color.accentColor)
						.accessibilityIdentifier("inbox.unmute")
					}
				}
			}
			.padding(EdgeInsets(top: 9, leading: 14, bottom: 9, trailing: 12))
		}
	}
}

/// A centred symbol and a line or two, for starting, failure and empty.
struct InboxNotice: View {
	let symbol: String
	let title: String
	let caption: String?

	var body: some View {
		VStack(spacing: 4) {
			Image(systemName: symbol)
				.font(.system(size: 28))
				.foregroundStyle(.tertiary)
			Text(title)
				.font(.system(size: 13, weight: .semibold))
				.padding(.top, 4)
			if let caption {
				Text(caption)
					.font(.system(size: 11))
					.foregroundStyle(.secondary)
					.multilineTextAlignment(.center)
			}
		}
		.frame(maxWidth: .infinity)
		.padding(.vertical, 36)
		.padding(.horizontal, 20)
		.accessibilityElement(children: .combine)
		.accessibilityIdentifier("inbox.notice")
	}
}
