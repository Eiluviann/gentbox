import Foundation
import Observation
import ServiceManagement
import os

/// Open at Login, through `SMAppService.mainApp` (architecture §3): the app
/// is the hub, so it should be running whenever you are logged in. It
/// registers itself once, on the first launch, and the menu's toggle changes
/// it after that.
@MainActor
@Observable
final class LoginItem {
	/// Whether the app opens at login.
	private(set) var isEnabled = SMAppService.mainApp.status == .enabled
	/// macOS wants your approval in System Settings › Login Items.
	private(set) var needsApproval = SMAppService.mainApp.status == .requiresApproval

	@ObservationIgnored private let defaults: UserDefaults
	@ObservationIgnored private let log = Logger(subsystem: "com.eiluviann.gentbox", category: "login-item")
	private static let didRegisterKey = "didRegisterLoginItem"

	init(defaults: UserDefaults = .standard) {
		self.defaults = defaults
	}

	/// Registers once, on the first launch; later launches leave your choice.
	/// UI tests launch with `-GentboxSkipLoginItem YES` and never register.
	func registerOnFirstLaunch() {
		guard !defaults.bool(forKey: Self.didRegisterKey), !defaults.bool(forKey: "GentboxSkipLoginItem") else {
			return
		}
		defaults.set(true, forKey: Self.didRegisterKey)
		setEnabled(true)
	}

	func setEnabled(_ isOn: Bool) {
		do {
			if isOn {
				try SMAppService.mainApp.register()
			} else {
				try SMAppService.mainApp.unregister()
			}
		} catch {
			log.error("login item change failed: \(error.localizedDescription, privacy: .public)")
		}
		refresh()
	}

	func refresh() {
		let status = SMAppService.mainApp.status
		isEnabled = status == .enabled
		needsApproval = status == .requiresApproval
	}
}
