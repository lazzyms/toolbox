// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "Toolbox",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "Toolbox", targets: ["Toolbox"]),
        .library(name: "ToolboxKit", targets: ["ToolboxKit"]),
    ],
    dependencies: [
        // Auto-updates. Sparkle verifies every download against the Ed25519
        // public key embedded in Info.plist, so a compromised GitHub account
        // can't push executable code to installed copies.
        .package(url: "https://github.com/sparkle-project/Sparkle", from: "2.9.5"),
        .package(url: "https://github.com/firebase/firebase-ios-sdk", from: "12.17.0"),
    ],
    targets: [
        // Pure processing logic. No UI, no global state — every entry point is a
        // synchronous file-in/file-out function so it can be called from any
        // concurrency domain and unit-tested without a running app.
        // Deliberately has no Sparkle dependency: the file tools stay offline.
        .target(name: "ToolboxKit"),
        .executableTarget(
            name: "Toolbox",
            dependencies: [
                "ToolboxKit",
                .product(name: "Sparkle", package: "Sparkle"),
                .product(name: "FirebaseAnalytics", package: "firebase-ios-sdk"),
                .product(name: "FirebaseCore", package: "firebase-ios-sdk"),
                .product(name: "FirebaseInstallations", package: "firebase-ios-sdk"),
            ],
            resources: [
                .process("Assets"),
                .process("Resources"),
            ]
        ),
        .testTarget(name: "ToolboxKitTests", dependencies: ["ToolboxKit"]),
        .testTarget(name: "ToolboxTests", dependencies: ["Toolbox"]),
    ]
)
