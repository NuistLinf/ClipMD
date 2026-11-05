import AppKit
import ClipMDCore
import SwiftUI

@MainActor
@Observable
final class HUDModel {
    enum Kind { case saved, working, failure }

    var visible = false
    var kind: Kind = .saved
    /// 图标从"文件"变成"对勾"
    var done = false
    var title = ""
    var subtitle = ""
    /// 文件在系统剪贴板上，才显示"⌘V 发送"
    var onClipboard = true
}

/// 屏幕顶部的保存提示。窗口只建一次，不显示时 orderOut，不参与合成。
@MainActor
final class HUDController {
    private let model = HUDModel()
    private lazy var panel = makePanel()
    private var generation = 0
    private static let size = NSSize(width: 520, height: 110)

    func show(_ feedback: Feedback) {
        generation += 1
        let current = generation
        let wasVisible = model.visible

        switch feedback {
        case .saved(let note, let fromHTML, let onClipboard):
            model.onClipboard = onClipboard
            model.kind = .saved
            model.title = "已保存为 Markdown"
            model.subtitle = note.url.lastPathComponent + (fromHTML ? " · 已保留网页格式" : "")
        case .copied(let title, let detail):
            model.kind = .saved
            model.onClipboard = true
            model.title = title
            model.subtitle = detail
        case .working(let title, let detail):
            model.kind = .working
            model.onClipboard = false
            model.title = title
            model.subtitle = detail
        case .failed(let failure):
            model.kind = .failure
            model.title = failure.title
            model.subtitle = failure.detail
        }
        model.done = false

        if !wasVisible {
            position()
            panel.orderFrontRegardless()
        }

        Task { [weak self] in
            // 先让隐藏状态渲染一帧，弹簧动画才有起点
            if !wasVisible { try? await Task.sleep(for: .milliseconds(16)) }
            guard let self, current == self.generation else { return }
            withAnimation(.spring(response: 0.42, dampingFraction: 0.68)) { self.model.visible = true }

            let hold: Duration
            switch self.model.kind {
            case .saved:
                try? await Task.sleep(for: .milliseconds(320))
                guard current == self.generation else { return }
                withAnimation(.snappy(duration: 0.3)) { self.model.done = true }
                // 手指在触控板上时会轻震一下
                NSHapticFeedbackManager.defaultPerformer.perform(.levelChange, performanceTime: .now)
                hold = .milliseconds(1800)
            case .working:
                // 一直显示到做完（被下一条提示替换）；万一卡住，一分钟后收起
                hold = .seconds(60)
            case .failure:
                NSSound.beep()
                // 出错时多停一会儿，留时间看清原因
                hold = .milliseconds(3500)
            }

            try? await Task.sleep(for: hold)
            guard current == self.generation else { return }
            withAnimation(.smooth(duration: 0.35)) { self.model.visible = false }

            try? await Task.sleep(for: .milliseconds(400))
            guard current == self.generation else { return }
            self.panel.orderOut(nil)
        }
    }

    /// 出现在鼠标所在屏幕的顶部中间（菜单栏下方）
    private func position() {
        let mouse = NSEvent.mouseLocation
        let screen = NSScreen.screens.first { NSMouseInRect(mouse, $0.frame, false) } ?? NSScreen.main ?? NSScreen.screens[0]
        let visible = screen.visibleFrame
        let size = Self.size
        panel.setFrame(NSRect(x: visible.midX - size.width / 2, y: visible.maxY - size.height,
                              width: size.width, height: size.height), display: false)
    }

    private func makePanel() -> NSPanel {
        let panel = NSPanel(contentRect: NSRect(origin: .zero, size: Self.size),
                            styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: true)
        panel.level = .statusBar
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary, .ignoresCycle]
        panel.ignoresMouseEvents = true
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false
        panel.isReleasedWhenClosed = false
        panel.animationBehavior = .none

        let hosting = NSHostingView(rootView: HUDView(model: model))
        hosting.sizingOptions = []
        panel.contentView = hosting
        return panel
    }
}

private struct HUDView: View {
    let model: HUDModel
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        HStack(spacing: 10) {
            ZStack {
                Circle().fill(tint.opacity(0.2))
                switch model.kind {
                case .saved: SwapIcon(from: "doc", to: "checkmark", swapped: model.done)
                case .working: ProgressView().controlSize(.small)
                case .failure: Image(systemName: "exclamationmark")
                }
            }
            .font(.system(size: 15, weight: .bold))
            .foregroundStyle(tint)
            .frame(width: 36, height: 36)

            VStack(alignment: .leading, spacing: 2) {
                Text(model.title)
                    .font(.system(size: 14, weight: .semibold))
                if !model.subtitle.isEmpty {
                    Text(model.subtitle)
                        .font(.system(size: 12))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
            }

            if model.kind == .saved, model.onClipboard {
                Text(model.title.hasPrefix("已保存") ? "⌘V 发送" : "⌘V 粘贴")
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 9)
                    .padding(.vertical, 4)
                    .background(.primary.opacity(0.07), in: Capsule())
                    .padding(.leading, 6)
            }
        }
        .padding(.leading, 10)
        .padding(.trailing, 16)
        .padding(.vertical, 10)
        .frame(maxWidth: 460)
        .fixedSize()
        .glassEffect(.regular, in: Capsule())
        .scaleEffect(model.visible || reduceMotion ? 1 : 0.86, anchor: .top)
        .offset(y: model.visible || reduceMotion ? 0 : -22)
        .blur(radius: model.visible || reduceMotion ? 0 : 6)
        .opacity(model.visible ? 1 : 0)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .padding(.top, 10)
        .accessibilityElement(children: .combine)
    }

    private var tint: Color {
        switch model.kind {
        case .saved: model.done ? .green : .accentColor
        case .working: .accentColor
        case .failure: .orange
        }
    }
}
