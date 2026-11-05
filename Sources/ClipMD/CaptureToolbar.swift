import AppKit
import SwiftUI

enum CaptureTool: String {
    case rectangle, ellipse, mosaic
}

/// 截图工具栏上的按钮
enum CaptureCommand {
    case tool(CaptureTool)
    case color(Int)
    case width(Int)
    case undo
    case recognize
    case pin
    case cancel
    case confirm
}

/// 当前选的工具、颜色和粗细。颜色和粗细记在偏好里，下次截图接着用
@MainActor
@Observable
final class CaptureToolState {
    var tool: CaptureTool?
    var canUndo = false
    var colorIndex: Int { didSet { UserDefaults.standard.set(colorIndex, forKey: "capture.color") } }
    var widthIndex: Int { didSet { UserDefaults.standard.set(widthIndex, forKey: "capture.width") } }

    static let colors: [NSColor] = [
        NSColor(srgbRed: 1.00, green: 0.23, blue: 0.19, alpha: 1),
        NSColor(srgbRed: 1.00, green: 0.58, blue: 0.00, alpha: 1),
        NSColor(srgbRed: 1.00, green: 0.80, blue: 0.00, alpha: 1),
        NSColor(srgbRed: 0.20, green: 0.78, blue: 0.35, alpha: 1),
        NSColor(srgbRed: 0.04, green: 0.52, blue: 1.00, alpha: 1),
        NSColor(srgbRed: 1.00, green: 1.00, blue: 1.00, alpha: 1),
        NSColor(srgbRed: 0.11, green: 0.11, blue: 0.12, alpha: 1),
    ]
    static let colorNames = ["红", "橙", "黄", "绿", "蓝", "白", "黑"]
    /// 矩形、椭圆的线宽（点）
    static let lineWidths: [CGFloat] = [2, 4, 7]
    /// 马赛克画笔的粗细（点）
    static let brushWidths: [CGFloat] = [12, 22, 36]
    static let widthNames = ["细", "中", "粗"]

    init() {
        let defaults = UserDefaults.standard
        colorIndex = min(max(defaults.integer(forKey: "capture.color"), 0), Self.colors.count - 1)
        widthIndex = defaults.object(forKey: "capture.width") as? Int ?? 1
    }

    var color: NSColor { Self.colors[colorIndex] }

    var width: CGFloat {
        let widths = tool == .mosaic ? Self.brushWidths : Self.lineWidths
        return widths[min(max(widthIndex, 0), widths.count - 1)]
    }
}

/// 截图选好区域后出现的工具栏：标注工具、撤销、识别文字、钉在桌面、丢弃、完成
struct CaptureToolbar: View {
    let state: CaptureToolState
    let perform: (CaptureCommand) -> Void

    var body: some View {
        VStack(alignment: .trailing, spacing: 6) {
            HStack(spacing: 2) {
                toolButton(.rectangle, symbol: "rectangle", help: "矩形（R）")
                toolButton(.ellipse, symbol: "circle", help: "椭圆（O）")
                toolButton(.mosaic, symbol: "checkerboard.rectangle", help: "马赛克（M）")
                separator
                CaptureToolButton(symbol: "arrow.uturn.backward", help: "撤销（⌘Z）", disabled: !state.canUndo) { perform(.undo) }
                separator
                CaptureToolButton(symbol: "text.viewfinder", help: "识别文字，放到剪贴板（T）") { perform(.recognize) }
                CaptureToolButton(symbol: "pin", help: "钉在桌面（P）") { perform(.pin) }
                separator
                CaptureToolButton(symbol: "xmark", help: "丢弃（Esc）") { perform(.cancel) }
                CaptureToolButton(symbol: "checkmark", help: "放到剪贴板（回车，或双击选区）", prominent: true) { perform(.confirm) }
            }
            .padding(4)
            .glassEffect(.regular, in: Capsule())

            if let tool = state.tool {
                options(for: tool)
                    .transition(.opacity.combined(with: .scale(scale: 0.92, anchor: .topTrailing)))
            }
        }
        .animation(.snappy(duration: 0.22), value: state.tool)
        .padding(1)
        .fixedSize()
    }

    private var separator: some View {
        Rectangle()
            .fill(.primary.opacity(0.15))
            .frame(width: 1, height: 18)
            .padding(.horizontal, 4)
    }

    private func toolButton(_ tool: CaptureTool, symbol: String, help: String) -> some View {
        CaptureToolButton(symbol: symbol, help: help, selected: state.tool == tool) { perform(.tool(tool)) }
    }

    /// 选了工具后出现的第二行：颜色（马赛克没有）和粗细
    private func options(for tool: CaptureTool) -> some View {
        HStack(spacing: 8) {
            if tool != .mosaic {
                ForEach(CaptureToolState.colors.indices, id: \.self) { index in
                    let selected = state.colorIndex == index
                    Button { perform(.color(index)) } label: {
                        Circle()
                            .fill(Color(nsColor: CaptureToolState.colors[index]))
                            .overlay(Circle().strokeBorder(.black.opacity(0.15), lineWidth: 0.5))
                            .frame(width: 16, height: 16)
                            .padding(3)
                            .overlay(Circle().strokeBorder(Color.accentColor, lineWidth: selected ? 2 : 0))
                            .contentShape(Circle())
                    }
                    .buttonStyle(.plain)
                    .help(CaptureToolState.colorNames[index])
                    .accessibilityLabel(CaptureToolState.colorNames[index])
                    .accessibilityAddTraits(selected ? .isSelected : [])
                }
                separator
            }
            ForEach(CaptureToolState.widthNames.indices, id: \.self) { index in
                let selected = state.widthIndex == index
                Button { perform(.width(index)) } label: {
                    Circle()
                        .fill(selected ? Color.accentColor : Color.primary.opacity(0.75))
                        .frame(width: 5 + CGFloat(index) * 4, height: 5 + CGFloat(index) * 4)
                        .frame(width: 24, height: 24)
                        .background(Circle().fill(selected ? Color.accentColor.opacity(0.16) : .clear))
                        .contentShape(Circle())
                }
                .buttonStyle(.plain)
                .help(CaptureToolState.widthNames[index])
                .accessibilityLabel(CaptureToolState.widthNames[index])
                .accessibilityAddTraits(selected ? .isSelected : [])
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 5)
        .glassEffect(.regular, in: Capsule())
    }
}

private struct CaptureToolButton: View {
    let symbol: String
    let help: String
    var selected = false
    var prominent = false
    var disabled = false
    let action: () -> Void

    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 14, weight: prominent ? .bold : .medium))
                .foregroundStyle(prominent ? AnyShapeStyle(.white) : selected ? AnyShapeStyle(Color.accentColor) : AnyShapeStyle(.primary))
                .frame(width: 32, height: 32)
                .background(Circle().fill(background))
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .disabled(disabled)
        .opacity(disabled ? 0.35 : 1)
        .onHover { hovering = $0 }
        .help(help)
        .accessibilityLabel(help)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    private var background: Color {
        if prominent { return hovering ? Color.accentColor.opacity(0.85) : Color.accentColor }
        if selected { return Color.accentColor.opacity(0.18) }
        return hovering && !disabled ? Color.primary.opacity(0.08) : .clear
    }
}

/// 工具栏的宿主视图：第一次点击就能按到按钮（截图层是刚弹出来的窗口）
final class CaptureToolbarHost: NSHostingView<CaptureToolbar> {
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
}
