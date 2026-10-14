// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "CTVSDK",
    platforms: [ .tvOS(.v14) ],
    products: [
        .library(name: "CTVSDK", targets: ["CTVSDK"]),
    ],
    targets: [
        .target(name: "CTVSDK", path: "Sources/CTVSDK"),
        .testTarget(name: "CTVSDKTests", dependencies: ["CTVSDK"], path: "Tests/CTVSDKTests"),
    ]
)
