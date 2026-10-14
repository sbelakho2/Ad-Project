// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "ApexAdapterDevKit",
    platforms: [
        .iOS(.v14), .tvOS(.v14)
    ],
    products: [
        .library(name: "ApexAdapterDevKit", targets: ["ApexAdapterDevKit"]),
        .executable(name: "apex-adapter-runner", targets: ["apex-adapter-runner"]),
    ],
    dependencies: [
        // Named local dependency: without the explicit name SwiftPM derives the
        // identity from the directory ("ios") and collides with this package's
        // own path component, producing a self-cycle.
        .package(name: "RivalApexMediationSDK", path: "../../core/ios")
    ],
    targets: [
        .target(
            name: "ApexAdapterDevKit",
            dependencies: [
                .product(name: "RivalApexMediationSDK", package: "RivalApexMediationSDK")
            ],
            path: "Sources/ApexAdapterDevKit"
        ),
        .executableTarget(
            name: "apex-adapter-runner",
            dependencies: ["ApexAdapterDevKit"],
            path: "Sources/apex-adapter-runner"
        ),
        .testTarget(
            name: "ApexAdapterDevKitTests",
            dependencies: ["ApexAdapterDevKit"],
            path: "Tests/ApexAdapterDevKitTests"
        ),
    ],
    swiftLanguageModes: [.v5]
)
