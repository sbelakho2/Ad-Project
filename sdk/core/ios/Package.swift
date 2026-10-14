// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "RivalApexMediationSDK",
    platforms: [
        .iOS(.v14),
        .tvOS(.v14),
        .macOS(.v10_15)
    ],
    products: [
        .library(
            name: "RivalApexMediationSDK",
            targets: ["RivalApexMediationSDK"]
        ),
    ],
    dependencies: [
        // Networking
        .package(url: "https://github.com/Alamofire/Alamofire.git", from: "5.8.0"),
        // Crypto
        .package(url: "https://github.com/apple/swift-crypto.git", from: "3.0.0"),
    ],
    targets: [
        .target(
            name: "RivalApexMediationSDK",
            dependencies: [
                "Alamofire",
                .product(name: "Crypto", package: "swift-crypto"),
            ],
            path: "Sources",
            swiftSettings: [
                .enableExperimentalFeature("StrictConcurrency")
            ]
        ),
        .testTarget(
            name: "RivalApexMediationSDKTests",
            dependencies: ["RivalApexMediationSDK"],
            path: "Tests",
            exclude: ["AppExtension"]
        ),
        .testTarget(
            name: "RivalApexMediationSDKAppExtensionTests",
            dependencies: ["RivalApexMediationSDK"],
            path: "Tests/AppExtension",
            swiftSettings: [
                .define("APP_EXTENSION")
            ]
        ),
    ]
)
