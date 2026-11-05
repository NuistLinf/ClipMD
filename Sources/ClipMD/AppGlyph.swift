import AppKit
import SwiftUI

/// ClipMD 的标志：截图取景框的四个角，中间是 Markdown 标志（M↓），和应用图标是同一个设计。
/// 悬浮窗左上角用彩色的 `AppGlyph`，菜单栏用单色的 `MenuBarGlyph`（模板图，系统按菜单栏的深浅自动上色）。

/// 取景框的四个角，`arm` 和 `radius` 是相对边长的比例
struct GlyphBrackets: Shape {
    var arm: CGFloat = 0.3
    var radius: CGFloat = 0.12

    func path(in rect: CGRect) -> Path {
        let side = min(rect.width, rect.height)
        let arm = side * arm, radius = side * radius
        let r = rect
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

/// M 加向下的箭头
private struct MarkdownGlyphMark: View {
    let size: CGFloat

    var body: some View {
        HStack(alignment: .center, spacing: size * 0.04) {
            Text("M")
                .font(.system(size: size, weight: .heavy, design: .rounded))
            Image(systemName: "arrow.down")
                .font(.system(size: size * 0.74, weight: .black))
                .offset(y: size * 0.03)
        }
        .fixedSize()
    }
}

/// 彩色的小标志。深色模式和应用图标一样是深色底；浅色模式换成浅色底、深色字，放在浅色玻璃上不突兀
struct AppGlyph: View {
    var size: CGFloat = 20
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        let dark = colorScheme == .dark
        let shape = RoundedRectangle(cornerRadius: size * 0.27, style: .continuous)
        ZStack {
            shape.fill(LinearGradient(colors: dark ? [Self.color(0x3A3F49), Self.color(0x16181C)] : [.white, Self.color(0xE4E9F1)],
                                      startPoint: .top, endPoint: .bottom))
            shape.strokeBorder(dark ? Color.white.opacity(0.16) : Color.black.opacity(0.1), lineWidth: 0.5)
            GlyphBrackets()
                .stroke(LinearGradient(colors: dark ? [Self.color(0x7DB6FF), Self.color(0x2F7BF5)] : [Self.color(0x4C93FF), Self.color(0x1F63E6)],
                                       startPoint: .top, endPoint: .bottom),
                        style: StrokeStyle(lineWidth: max(size * 0.075, 1.2), lineCap: .round, lineJoin: .round))
                .padding(size * 0.17)
            MarkdownGlyphMark(size: size * 0.3)
                .foregroundStyle(dark ? Color.white : Self.color(0x1E2633))
        }
        .frame(width: size, height: size)
        .shadow(color: .black.opacity(dark ? 0.3 : 0.12), radius: size * 0.08, y: size * 0.04)
        .accessibilityHidden(true)
    }

    static func color(_ hex: UInt32) -> Color {
        Color(.sRGB, red: Double((hex >> 16) & 0xFF) / 255, green: Double((hex >> 8) & 0xFF) / 255, blue: Double(hex & 0xFF) / 255)
    }
}

/// 菜单栏用的单色标志：只有取景框和 M↓，没有底板
struct MenuBarGlyph: View {
    var size: CGFloat = 18

    var body: some View {
        ZStack {
            GlyphBrackets(arm: 0.3, radius: 0.16)
                .stroke(.black, style: StrokeStyle(lineWidth: 1.5, lineCap: .round, lineJoin: .round))
                .padding(1)
            MarkdownGlyphMark(size: size * 0.4)
                .foregroundStyle(.black)
        }
        .frame(width: size, height: size)
    }

    /// 模板图：系统会按菜单栏的深浅、是否高亮自动上色
    @MainActor
    static func templateImage(size: CGFloat = 18) -> NSImage? {
        let renderer = ImageRenderer(content: MenuBarGlyph(size: size))
        renderer.scale = 2
        guard let cgImage = renderer.cgImage else { return nil }
        let image = NSImage(cgImage: cgImage, size: NSSize(width: size, height: size))
        image.isTemplate = true
        image.accessibilityDescription = "ClipMD"
        return image
    }
}
