// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "Toolbox",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "Toolbox", targets: ["Toolbox"]),
        .library(name: "ToolboxKit", targets: ["ToolboxKit"]),
    ],
    targets: [
        // Pure processing logic. No UI, no global state — every entry point is a
        // synchronous file-in/file-out function so it can be called from any
        // concurrency domain and unit-tested without a running app.
        .target(name: "ToolboxKit"),
        .executableTarget(name: "Toolbox", dependencies: ["ToolboxKit"]),
        .testTarget(name: "ToolboxKitTests", dependencies: ["ToolboxKit"]),
    ]
)
