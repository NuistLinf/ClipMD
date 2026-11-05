import AppKit
import SwiftUI

// 画 ClipMD 的应用图标（1024 × 1024 PNG）。
// 按苹果 macOS 图标模板：1024 画布里放 824 × 824 的圆角方形主体，四周留 100 给阴影。
//
//     swiftc -parse-as-library -O scripts/icon/IconRenderer.swift -o /tmp/icon-renderer
//     /tmp/icon-renderer <输出文件夹> [方案名...]

enum Template {
    static let canvas: CGFloat = 1024
    static let body: CGFloat = 824
    static let radius: CGFloat = 185.4
}

extension Color {
    init(hex: UInt32, opacity: Double = 1) {
        self.init(.sRGB, red: Double((hex >> 16) & 0xFF) / 255, green: Double((hex >> 8) & 0xFF) / 255,
                  blue: Double(hex & 0xFF) / 255, opacity: opacity)
    }
}

/// 圆角方形的底板：渐变、左上角的柔光、内侧一圈细高光，外面是标准的投影
struct Squircle<Content: View>: View {
    let top: Color
    let bottom: Color
    var glow: Double = 0.22
    var rim: Double = 0.35
    @ViewBuilder let content: Content

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: Template.radius, style: .continuous)
        ZStack {
            shape.fill(LinearGradient(colors: [top, bottom], startPoint: .top, endPoint: .bottom))
            shape.fill(RadialGradient(colors: [.white.opacity(glow), .clear], center: UnitPoint(x: 0.28, y: 0.12),
                                      startRadius: 0, endRadius: 620))
            content
            shape.strokeBorder(LinearGradient(colors: [.white.opacity(rim), .white.opacity(0.04), .black.opacity(0.12)],
                                              startPoint: .top, endPoint: .bottom), lineWidth: 3)
        }
        .frame(width: Template.body, height: Template.body)
        .clipShape(shape)
        .shadow(color: .black.opacity(0.16), radius: 2, y: 1)
        .shadow(color: .black.opacity(0.28), radius: 16, y: 12)
        .frame(width: Template.canvas, height: Template.canvas)
    }
}

/// Markdown 的标志：M 加一个向下的箭头
struct MarkdownMark: View {
    var size: CGFloat
    var color: AnyShapeStyle

    var body: some View {
        HStack(alignment: .center, spacing: size * 0.06) {
            Text("M")
                .font(.system(size: size, weight: .heavy, design: .rounded))
            Image(systemName: "arrow.down")
                .font(.system(size: size * 0.74, weight: .black))
                .offset(y: size * 0.04)
        }
        .foregroundStyle(color)
    }
}

/// 剪贴板的金属夹子：上亮下暗的拉丝金属，顶边一道高光，孔有内阴影
struct Clip: View {
    var width: CGFloat = 300
    var glass = false

    var body: some View {
        let height = width * 0.34
        let shape = RoundedRectangle(cornerRadius: height * 0.36, style: .continuous)
        ZStack {
            if glass {
                shape.fill(.white.opacity(0.32))
                shape.fill(LinearGradient(colors: [.white.opacity(0.45), .white.opacity(0.05)], startPoint: .top, endPoint: .bottom))
            } else {
                shape.fill(LinearGradient(stops: [
                    .init(color: Color(hex: 0xFDFEFF), location: 0),
                    .init(color: Color(hex: 0xE3E8EF), location: 0.35),
                    .init(color: Color(hex: 0xB9C2CF), location: 0.75),
                    .init(color: Color(hex: 0x98A2B1), location: 1),
                ], startPoint: .top, endPoint: .bottom))
                // 一道斜向的反光
                shape.fill(LinearGradient(colors: [.clear, .white.opacity(0.55), .clear], startPoint: UnitPoint(x: 0.2, y: 0), endPoint: UnitPoint(x: 0.55, y: 1)))
                    .blendMode(.screen)
            }
            shape.strokeBorder(LinearGradient(colors: [.white.opacity(0.95), .white.opacity(0.2), .black.opacity(0.22)],
                                              startPoint: .top, endPoint: .bottom), lineWidth: 3)
            // 夹子上的孔
            Capsule()
                .fill(LinearGradient(colors: glass ? [Color(hex: 0x10307E, opacity: 0.7), Color(hex: 0x2A55C8, opacity: 0.5)]
                                                   : [Color(hex: 0x4A5361), Color(hex: 0x7D8796)],
                                     startPoint: .top, endPoint: .bottom))
                .overlay(Capsule().strokeBorder(LinearGradient(colors: [.black.opacity(0.35), .white.opacity(0.6)], startPoint: .top, endPoint: .bottom),
                                                lineWidth: 2.5))
                .frame(width: width * 0.32, height: height * 0.28)
                .offset(y: -height * 0.1)
        }
        .frame(width: width, height: height)
        .shadow(color: .black.opacity(0.3), radius: 10, y: 7)
    }
}

/// 方案 A：蓝色底板上一张白纸，夹着金属夹子，纸上是 Markdown 标志
struct PaperIcon: View {
    var body: some View {
        Squircle(top: Color(hex: 0x5B9BFF), bottom: Color(hex: 0x1D4FD6)) {
            ZStack {
                RoundedRectangle(cornerRadius: 46, style: .continuous)
                    .fill(LinearGradient(colors: [.white, Color(hex: 0xEEF2F8)], startPoint: .top, endPoint: .bottom))
                    .overlay(RoundedRectangle(cornerRadius: 46, style: .continuous).strokeBorder(.white, lineWidth: 2))
                    .frame(width: 500, height: 590)
                    .shadow(color: Color(hex: 0x0B2A7A, opacity: 0.45), radius: 26, y: 18)
                    .offset(y: 46)
                VStack(spacing: 34) {
                    MarkdownMark(size: 200, color: AnyShapeStyle(LinearGradient(colors: [Color(hex: 0x3B82F6), Color(hex: 0x1D4ED8)],
                                                                                    startPoint: .top, endPoint: .bottom)))
                    VStack(alignment: .leading, spacing: 22) {
                        Capsule().fill(Color(hex: 0xC9D3E3)).frame(width: 300, height: 22)
                        Capsule().fill(Color(hex: 0xDCE3EE)).frame(width: 220, height: 22)
                    }
                    .frame(width: 300, alignment: .leading)
                }
                .offset(y: 84)
                Clip(width: 300)
                    .offset(y: -244)
            }
        }
    }
}

/// 方案 B：液态玻璃。蓝色底板上有柔和的彩色光斑，前面一块磨砂玻璃板（透出后面模糊的光斑），和 ClipMD 的界面一个味道
struct LiquidGlassIcon: View {
    private static let sheet = CGSize(width: 520, height: 590)
    private static let sheetOffset: CGFloat = 46

    var body: some View {
        Squircle(top: Color(hex: 0x3B74FF), bottom: Color(hex: 0x1435B0), glow: 0.16, rim: 0.4) {
            ZStack {
                Orbs().blur(radius: 70)
                // 玻璃板里看到的是更模糊、更亮的光斑
                Orbs()
                    .blur(radius: 46)
                    .brightness(0.08)
                    .mask(sheetShape.frame(width: Self.sheet.width, height: Self.sheet.height).offset(y: Self.sheetOffset))
                ZStack {
                    sheetShape.fill(.white.opacity(0.16))
                    sheetShape.fill(LinearGradient(colors: [.white.opacity(0.32), .white.opacity(0.04), .clear],
                                                   startPoint: .top, endPoint: .center))
                    sheetShape.strokeBorder(LinearGradient(colors: [.white.opacity(0.95), .white.opacity(0.25), .white.opacity(0.5)],
                                                           startPoint: .topLeading, endPoint: .bottomTrailing), lineWidth: 3.5)
                }
                .frame(width: Self.sheet.width, height: Self.sheet.height)
                .shadow(color: Color(hex: 0x07174F, opacity: 0.35), radius: 28, y: 20)
                .offset(y: Self.sheetOffset)
                VStack(spacing: 40) {
                    MarkdownMark(size: 212, color: AnyShapeStyle(.white))
                        .shadow(color: Color(hex: 0x0A1F6B, opacity: 0.35), radius: 10, y: 6)
                    VStack(alignment: .leading, spacing: 22) {
                        Capsule().fill(.white.opacity(0.55)).frame(width: 300, height: 22)
                        Capsule().fill(.white.opacity(0.35)).frame(width: 220, height: 22)
                    }
                    .frame(width: 300, alignment: .leading)
                }
                .offset(y: 86)
                Clip(width: 290)
                    .offset(y: -Self.sheet.height / 2 + Self.sheetOffset - 6)
            }
        }
    }

    private var sheetShape: RoundedRectangle { RoundedRectangle(cornerRadius: 58, style: .continuous) }
}

/// 底板上的彩色光斑
struct Orbs: View {
    var body: some View {
        ZStack {
            Circle().fill(Color(hex: 0x3DDCFF, opacity: 0.85)).frame(width: 420).offset(x: -230, y: -190)
            Circle().fill(Color(hex: 0x9B6BFF, opacity: 0.85)).frame(width: 480).offset(x: 250, y: 250)
            Circle().fill(Color(hex: 0x6FB6FF, opacity: 0.8)).frame(width: 300).offset(x: 240, y: -250)
        }
        .frame(width: Template.body, height: Template.body)
    }
}

/// 方案 C：深色底板上的截图取景框，中间是 Markdown 标志（截图 + Markdown）
struct CaptureIcon: View {
    var body: some View {
        Squircle(top: Color(hex: 0x353A44), bottom: Color(hex: 0x14161A), glow: 0.12, rim: 0.22) {
            ZStack {
                CornerBrackets(size: 560, arm: 150, line: 46)
                    .stroke(LinearGradient(colors: [Color(hex: 0x7DB6FF), Color(hex: 0x2F7BF5)], startPoint: .top, endPoint: .bottom),
                            style: StrokeStyle(lineWidth: 46, lineCap: .round, lineJoin: .round))
                    .frame(width: 560, height: 560)
                    .shadow(color: Color(hex: 0x2F7BF5, opacity: 0.55), radius: 22)
                MarkdownMark(size: 210, color: AnyShapeStyle(LinearGradient(colors: [.white, Color(hex: 0xD6E2F5)],
                                                                            startPoint: .top, endPoint: .bottom)))
                    .shadow(color: .black.opacity(0.4), radius: 12, y: 8)
            }
        }
    }
}

/// 截图取景框的四个角
struct CornerBrackets: Shape {
    let size: CGFloat
    let arm: CGFloat
    let line: CGFloat

    func path(in rect: CGRect) -> Path {
        let r = rect.insetBy(dx: line / 2, dy: line / 2)
        let radius: CGFloat = 60
        var path = Path()
        // 左上
        path.move(to: CGPoint(x: r.minX, y: r.minY + arm))
        path.addLine(to: CGPoint(x: r.minX, y: r.minY + radius))
        path.addQuadCurve(to: CGPoint(x: r.minX + radius, y: r.minY), control: CGPoint(x: r.minX, y: r.minY))
        path.addLine(to: CGPoint(x: r.minX + arm, y: r.minY))
        // 右上
        path.move(to: CGPoint(x: r.maxX - arm, y: r.minY))
        path.addLine(to: CGPoint(x: r.maxX - radius, y: r.minY))
        path.addQuadCurve(to: CGPoint(x: r.maxX, y: r.minY + radius), control: CGPoint(x: r.maxX, y: r.minY))
        path.addLine(to: CGPoint(x: r.maxX, y: r.minY + arm))
        // 右下
        path.move(to: CGPoint(x: r.maxX, y: r.maxY - arm))
        path.addLine(to: CGPoint(x: r.maxX, y: r.maxY - radius))
        path.addQuadCurve(to: CGPoint(x: r.maxX - radius, y: r.maxY), control: CGPoint(x: r.maxX, y: r.maxY))
        path.addLine(to: CGPoint(x: r.maxX - arm, y: r.maxY))
        // 左下
        path.move(to: CGPoint(x: r.minX + arm, y: r.maxY))
        path.addLine(to: CGPoint(x: r.minX + radius, y: r.maxY))
        path.addQuadCurve(to: CGPoint(x: r.minX, y: r.maxY - radius), control: CGPoint(x: r.minX, y: r.maxY))
        path.addLine(to: CGPoint(x: r.minX, y: r.maxY - arm))
        return path
    }
}

@main
@MainActor
struct IconRendererMain {
    static let designs: [(String, AnyView)] = [
        ("paper", AnyView(PaperIcon())),
        ("glass", AnyView(LiquidGlassIcon())),
        ("capture", AnyView(CaptureIcon())),
    ]

    /// 对比图：每个方案在浅色、深色背景上的 256 / 128 / 64 / 32 / 16 点
    static func renderSheet(in folder: URL, names: [String]) {
        let sizes: [CGFloat] = [256, 128, 64, 32, 16]
        let images = names.compactMap { name in NSImage(contentsOf: folder.appending(path: "\(name).png")).map { (name, $0) } }
        let view = VStack(alignment: .leading, spacing: 0) {
            ForEach(images, id: \.0) { name, image in
                HStack(spacing: 0) {
                    ForEach([Color(hex: 0xECECEE), Color(hex: 0x1E1F22)], id: \.self) { background in
                        HStack(alignment: .bottom, spacing: 22) {
                            ForEach(sizes, id: \.self) { size in
                                Image(nsImage: image).resizable().interpolation(.high).frame(width: size, height: size)
                            }
                        }
                        .padding(24)
                        .background(background)
                    }
                }
            }
        }
        let renderer = ImageRenderer(content: view)
        renderer.scale = 2
        guard let cg = renderer.cgImage, let png = NSBitmapImageRep(cgImage: cg).representation(using: .png, properties: [:]) else { return }
        try? png.write(to: folder.appending(path: "sheet.png"))
        print("sheet \(cg.width)x\(cg.height)")
    }

    static func main() {
        let arguments = CommandLine.arguments
        guard arguments.count >= 2 else {
            print("用法：icon-renderer <输出文件夹> [方案名...]")
            exit(1)
        }
        let folder = URL(filePath: arguments[1], directoryHint: .isDirectory)
        if arguments.count >= 3, arguments[2] == "sheet" {
            renderSheet(in: folder, names: Array(arguments.dropFirst(3)))
            return
        }
        let wanted = Set(arguments.dropFirst(2))
        for (name, view) in designs where wanted.isEmpty || wanted.contains(name) {
            let renderer = ImageRenderer(content: view.frame(width: Template.canvas, height: Template.canvas))
            renderer.scale = 1
            guard let image = renderer.cgImage,
                  let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]) else {
                print("画不出 \(name)")
                continue
            }
            let url = folder.appending(path: "\(name).png")
            try? png.write(to: url)
            print("\(url.path(percentEncoded: false)) \(image.width)x\(image.height)")
        }
    }
}
