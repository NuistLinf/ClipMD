// swift-tools-version: 6.2
import PackageDescription

let package = Package(
    name: "ClipMD",
    platforms: [.macOS(.v26)],
    targets: [
        // 纯逻辑：命名、写文件、读最近记录、Markdown 分块。不依赖 AppKit，方便测试
        .target(name: "ClipMDCore"),
        // 预览里的 LaTeX 公式：原生绘制，不用网页。
        // 第三方 SwiftMath 1.7.3（MIT）的源码副本，只改了字体的查找位置，见 Vendor/SwiftMath/VENDOR.md
        .target(
            name: "SwiftMath",
            path: "Vendor/SwiftMath/Sources/SwiftMath",
            swiftSettings: [.swiftLanguageMode(.v5), .unsafeFlags(["-suppress-warnings"])]
        ),
        .executableTarget(
            name: "ClipMD",
            dependencies: ["ClipMDCore", "SwiftMath"],
            linkerSettings: [.linkedFramework("Carbon")]
        ),
        .testTarget(name: "ClipMDCoreTests", dependencies: ["ClipMDCore"]),
    ]
)
