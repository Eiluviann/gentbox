// swift-tools-version: 6.0
// The Gentbox core for Swift: the UniFFI XCFramework (built by
// `scripts/build-xcframework.sh`, gitignored), the generated bindings, and
// the main-actor models the app's views use (`InboxModel`, `PresenceMonitor`).
// Views never call the FFI directly.
import PackageDescription

let package = Package(
	name: "GentboxCore",
	platforms: [.macOS(.v15)],
	products: [
		.library(name: "GentboxCore", targets: ["GentboxCore"]),
	],
	targets: [
		.binaryTarget(name: "GentboxCoreFFI", path: "GentboxCoreFFI.xcframework"),
		.target(
			name: "GentboxCore",
			dependencies: ["GentboxCoreFFI"],
			path: "Sources/GentboxCore",
			// The generated bindings predate Swift 6's strict concurrency in
			// places; the hand-written files are checked under it in full.
			swiftSettings: [.swiftLanguageMode(.v6)],
		),
		.testTarget(
			name: "GentboxCoreTests",
			dependencies: ["GentboxCore"],
			path: "Tests/GentboxCoreTests",
		),
	],
)
