// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "Feed",
    platforms: [
        .macOS(.v14),
        .iOS(.v17),
    ],
    products: [
        .library(name: "Feed", targets: ["Feed"]),
    ],
    targets: [
        .target(
            name: "Feed",
            swiftSettings: [
                .enableUpcomingFeature("ExistentialAny"),
                .enableExperimentalFeature("StrictConcurrency"),
            ]
        ),
        .testTarget(name: "FeedTests", dependencies: ["Feed"]),
    ]
)
