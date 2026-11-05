import ClipMDCore
import SwiftUI

enum DockEdge { case left, right }

enum PanelMetrics {
    static let width: CGFloat = 300
    /// 玻璃外面留出的透明边，给阴影用
    static let margin: CGFloat = 18
    static let cornerRadius: CGFloat = 22
    /// 收起后露在屏幕边上的宽度
    static let sliver: CGFloat = 6
    /// 列表每一行的高度。两个标签的行一样高，切换标签时窗口不用变高变矮
    static let rowHeight: CGFloat = 46
    /// 探出来时离屏幕边的距离
    static let peekInset: CGFloat = 10
}

@MainActor
@Observable
final class PanelModel {
    /// 收起时贴在哪条边；展开或探出时为 nil
    var tuckedEdge: DockEdge?
    /// 固定位置后不能拖动，也不会拖到边上自动收起
    var pinned = false
    var hint: ClipboardHint = .empty
    /// 刚保存成功，保存按钮短暂变成"已保存"
    var justSaved = false
    /// 刚保存的文件有没有放到系统剪贴板（拖进来保存的不放）
    var savedOnClipboard = true
    /// 正拖着东西经过悬浮窗
    var dropHover = DropHover.none
    /// 列表显示"已保存"还是"剪贴板"记录
    var tab = PanelTab.saved
    /// 剪贴板记录展开显示全部
    /// 列表展开（露出 10 条）还是收起（露出 5 条）；两个标签共用
    var listExpanded = false
    /// 列表滚到了哪里（测试时看滚轮有没有生效）
    @ObservationIgnored var listScrollOffset: CGFloat = 0
    /// 测试快照时为 true：ImageRenderer 画不出滚动视图里的内容，改成直接排出露出来的那几行
    @ObservationIgnored var snapshotMode = false
    /// 系统允许 ClipMD 随时读剪贴板（剪贴板记录要靠这个）
    var canReadClipboard = true
    /// 刚复制了一段文字，保存按钮提示可以点它保存
    var freshCopy = false
    /// 每复制一次加一，触发按钮弹一下
    var copyPulse = 0
    var failure: Failure?
    /// 删除后的"可撤销"提示
    var notice: Notice?
    /// macOS 每次都会询问是否允许读取剪贴板
    var pasteAsksEveryTime = false
}

enum PanelTab: String {
    case saved, clipboard
}

struct Notice: Equatable {
    let id = UUID()
    let message: String
}

struct PanelActions {
    var save: @MainActor () -> Void
    var copy: @MainActor (SavedNote) -> Bool
    var preview: @MainActor (SavedNote) -> Void
    var reveal: @MainActor (SavedNote) -> Void
    var hide: @MainActor (SavedNote) -> Void
    var trash: @MainActor (SavedNote) -> Void
    var undo: @MainActor () -> Void
    var dismissNotice: @MainActor () -> Void
    var openFolder: @MainActor () -> Void
    var toggleTuck: @MainActor () -> Void
    var togglePin: @MainActor () -> Void
    var dragChanged: @MainActor () -> Void
    var dragEnded: @MainActor () -> Void
    var fix: @MainActor (FailureFix) -> Void
    var openSettings: @MainActor () -> Void
    var selectTab: @MainActor (PanelTab) -> Void
    var copyClip: @MainActor (ClipItem) -> Void
    var previewClip: @MainActor (ClipItem) -> Void
    var saveClip: @MainActor (ClipItem) -> Void
    var recognizeClip: @MainActor (ClipItem) -> Void
    var revealClip: @MainActor (ClipItem) -> Void
    /// true：截图后识别文字
    var screenshot: @MainActor (Bool) -> Void
    var removeClip: @MainActor (ClipItem) -> Void
    var clearClips: @MainActor () -> Void
    var dismissFailure: @MainActor () -> Void
    var heightChanged: @MainActor (CGFloat) -> Void
}

struct PanelView: View {
    let store: NoteStore
    let history: ClipHistory
    let model: PanelModel
    let actions: PanelActions

    var body: some View {
        VStack(spacing: 0) {
            header

            SaveButton(model: model, action: actions.save)
                .padding(.horizontal, 10)

            if let failure = model.failure {
                FailureBanner(failure: failure, fix: actions.fix, dismiss: actions.dismissFailure)
                    .padding(.horizontal, 10)
                    .padding(.top, 8)
                    .transition(.move(edge: .top).combined(with: .opacity))
            } else if let notice = model.notice {
                NoticeBanner(notice: notice, undo: actions.undo, dismiss: actions.dismissNotice)
                    .padding(.horizontal, 10)
                    .padding(.top, 8)
                    .transition(.move(edge: .top).combined(with: .opacity))
            } else if model.pasteAsksEveryTime {
                PasteTip(fix: actions.fix)
                    .padding(.horizontal, 10)
                    .padding(.top, 8)
                    .transition(.opacity)
            }

            ListTabs(selection: model.tab, clipCount: history.items.count, select: actions.selectTab,
                     clear: model.tab == .clipboard && !history.items.isEmpty ? actions.clearClips : nil)
                .padding(.horizontal, 12)
                .padding(.top, 10)
                .padding(.bottom, 4)

            // 两个标签叠在一起，面板高度取两者中高的那个：切换标签时窗口不用变高变矮，
            // 只是内容左右滑动着淡入。没选中的标签只留一块同样高的空白（不建真正的列表），
            // 免得藏在后面的行（里面有 AppKit 视图）接到点击和滚轮
            ZStack(alignment: .top) {
                tabContent(.saved) { savedList }
                tabContent(.clipboard) { clipList }
            }
        }
        // 拖着文字经过时，底下的内容淡出去，只留玻璃和提示，免得字叠在一起
        .opacity(model.dropHover == .none ? 1 : 0.06)
        .frame(width: PanelMetrics.width)
        .fixedSize(horizontal: false, vertical: true)
        .glassEffect(.regular, in: RoundedRectangle(cornerRadius: PanelMetrics.cornerRadius, style: .continuous))
        .modifier(PanelChrome(model: model, actions: actions))
    }

    private func tabContent(_ tab: PanelTab, @ViewBuilder content: () -> some View) -> some View {
        let selected = model.tab == tab
        // 「已保存」在左、「剪贴板」在右，滑动方向和标签的位置一致
        let side: CGFloat = tab == .saved ? -1 : 1
        return content()
            .opacity(selected ? 1 : 0)
            .offset(x: selected ? 0 : side * 18)
            .allowsHitTesting(selected)
            .accessibilityHidden(!selected)
    }

    @ViewBuilder
    private var savedList: some View {
        if store.notes.isEmpty {
            EmptyState()
                .transition(.opacity)
        } else {
            ScrollingList(items: store.notes, model: model, active: model.tab == .saved) { note in
                NoteRow(note: note, now: store.now, actions: actions)
                    .transition(.asymmetric(
                        insertion: .move(edge: .top)
                            .combined(with: .opacity)
                            .combined(with: .scale(scale: 0.96, anchor: .top)),
                        removal: .opacity.combined(with: .scale(scale: 0.96))
                    ))
            }
        }
    }

    /// 剪贴板记录
    @ViewBuilder
    private var clipList: some View {
        let items = history.items
        if !history.isEnabled {
            ClipEmptyState(title: "剪贴板记录已关闭", detail: "在设置里打开「记录剪贴板」", button: "打开设置", action: actions.openSettings)
        } else if items.isEmpty, !model.canReadClipboard {
            ClipEmptyState(title: "要记录复制的内容，需要允许读取剪贴板", detail: "在「隐私与安全性 → 粘贴」里把 ClipMD 设成「允许」",
                           button: "打开系统设置") { actions.fix(.pasteSettings) }
        } else if items.isEmpty {
            ClipEmptyState(title: "复制过的文字会记在这里", detail: "不会存成文件，需要时点 ↓ 存成 md", button: nil, action: {})
        } else {
            ScrollingList(items: items, model: model, active: model.tab == .clipboard) { item in
                ClipRow(item: item, imageURL: history.imageURL(for: item), now: store.now, actions: actions)
                    .transition(.asymmetric(
                        insertion: .move(edge: .top).combined(with: .opacity),
                        removal: .opacity.combined(with: .scale(scale: 0.96))
                    ))
            }
        }
    }
}

/// 两个标签共用的列表：可以用滚轮翻看全部；平时露出 5 条，展开后露出 10 条。
/// 只建露出来的那几行（LazyVStack）
private struct ScrollingList<Item: Identifiable, Row: View>: View {
    let items: [Item]
    let model: PanelModel
    /// 当前标签的列表；另一个标签只占一块同样高的空白
    let active: Bool

    static var footerHeight: CGFloat { 26 }

    /// 列表占的总高度（含底部的「展开」和边距）。算出来的，没选中的标签不用真的建出列表
    static func height(count: Int, expanded: Bool) -> CGFloat {
        let visible = min(count, expanded ? 10 : 5)
        let rows = CGFloat(visible) * PanelMetrics.rowHeight + CGFloat(max(visible - 1, 0)) * 2
        return rows + (count > 5 ? footerHeight : 0) + 6
    }
    @ViewBuilder let row: (Item) -> Row

    @State private var scroll = ListScrollState()

    var body: some View {
        if active {
            list
        } else {
            Color.clear.frame(height: Self.height(count: items.count, expanded: model.listExpanded))
        }
    }

    private var list: some View {
        let visible = min(items.count, model.listExpanded ? 10 : 5)
        return VStack(spacing: 0) {
            if model.snapshotMode {
                rows(Array(items.prefix(visible)))
            } else {
                ScrollView {
                    rows(items)
                }
                // 系统的滚动条（尤其是接了鼠标时常驻的那种，带白色轨道）压在玻璃上很突兀，换成自己画的细条
                .scrollIndicators(.never)
                .scrollBounceBehavior(.basedOnSize)
                .frame(height: CGFloat(visible) * PanelMetrics.rowHeight + CGFloat(max(visible - 1, 0)) * 2)
                .onScrollGeometryChange(for: ScrollMetrics.self) {
                    ScrollMetrics(offset: $0.contentOffset.y, content: $0.contentSize.height, container: $0.containerSize.height)
                } action: { old, new in
                    scroll.metrics = new
                    model.listScrollOffset = new.offset
                    if old.offset != new.offset { scroll.flash() }
                }
                .mask { ListEdgeFade(scroll: scroll) }
                .overlay(alignment: .topTrailing) { ListScrollIndicator(scroll: scroll) }
                .onHover { if $0 { scroll.flash() } }
            }
            if items.count > 5 {
                Button(model.listExpanded ? "收起" : "展开（共 \(items.count) 条）") {
                    withAnimation(.snappy(duration: 0.3)) { model.listExpanded.toggle() }
                }
                .buttonStyle(.plain)
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(.secondary)
                .frame(height: Self.footerHeight)
            }
        }
        .padding(.horizontal, 6)
        .padding(.bottom, 6)
    }

    private func rows(_ items: [Item]) -> some View {
        LazyVStack(spacing: 2) {
            ForEach(items) { item in row(item) }
        }
    }
}

private struct ScrollMetrics: Equatable {
    var offset: CGFloat = 0
    var content: CGFloat = 0
    var container: CGFloat = 0
}

/// 列表滚动的状态。单独放在一个对象里，只有细条和边缘渐隐读它：
/// 滚动时只重画这两个小视图，不会让整个列表重新求值
@MainActor
@Observable
private final class ListScrollState {
    var metrics = ScrollMetrics()
    var indicatorVisible = false
    @ObservationIgnored private var hideTask: Task<Void, Never>?

    var scrollable: Bool { metrics.container > 0 && metrics.content > metrics.container + 1 }
    var hasMoreAbove: Bool { scrollable && metrics.offset > 1 }
    var hasMoreBelow: Bool { scrollable && metrics.offset < metrics.content - metrics.container - 1 }

    /// 滚动或鼠标移进列表时浮现，停下来约 1 秒后淡出
    func flash() {
        guard scrollable else { return }
        if !indicatorVisible { withAnimation(.easeOut(duration: 0.15)) { indicatorVisible = true } }
        hideTask?.cancel()
        hideTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(1000))
            guard !Task.isCancelled else { return }
            withAnimation(.easeOut(duration: 0.45)) { self?.indicatorVisible = false }
        }
    }
}

/// 贴在列表右边、玻璃边缘里面的一根细条，只在滚动时出现
private struct ListScrollIndicator: View {
    let scroll: ListScrollState

    var body: some View {
        let metrics = scroll.metrics
        if scroll.scrollable {
            let height = max(metrics.container * metrics.container / metrics.content, 24)
            let travel = metrics.container - height
            let progress = min(max(metrics.offset / (metrics.content - metrics.container), 0), 1)
            Capsule()
                .fill(.primary.opacity(0.22))
                .frame(width: 3, height: height)
                .offset(x: 3.5, y: travel * progress)
                .opacity(scroll.indicatorVisible ? 1 : 0)
                .allowsHitTesting(false)
                .accessibilityHidden(true)
        }
    }
}

/// 上面或下面还有内容时，列表那一边淡出一点，提示还能滚
private struct ListEdgeFade: View {
    let scroll: ListScrollState
    private static let fade: CGFloat = 14

    var body: some View {
        VStack(spacing: 0) {
            LinearGradient(colors: [.black.opacity(scroll.hasMoreAbove ? 0 : 1), .black], startPoint: .top, endPoint: .bottom)
                .frame(height: Self.fade)
            Color.black
            LinearGradient(colors: [.black, .black.opacity(scroll.hasMoreBelow ? 0 : 1)], startPoint: .top, endPoint: .bottom)
                .frame(height: Self.fade)
        }
        .animation(.easeOut(duration: 0.2), value: scroll.hasMoreAbove)
        .animation(.easeOut(duration: 0.2), value: scroll.hasMoreBelow)
    }
}

/// 玻璃外面的东西：拖放提示、收起时的把手、高度测量、透明边
private struct PanelChrome: ViewModifier {
    let model: PanelModel
    let actions: PanelActions

    func body(content: Content) -> some View {
        content
        .overlay {
            switch model.dropHover {
            case .accept(let action):
                DropZones(active: action)
                    .transition(.opacity.combined(with: .scale(scale: 0.97)))
            case .reject(let message):
                DropOverlay(title: "这个保存不了", detail: message)
                    .transition(.opacity)
            case .none:
                EmptyView()
            }
        }
        .scaleEffect(model.dropHover.isAccepting ? 1.015 : 1)
        .overlay(alignment: model.tuckedEdge == .left ? .trailing : .leading) {
            // 收起后露出的那条细边上的把手
            Capsule()
                .fill(Color.accentColor.opacity(0.75))
                .frame(width: 3, height: 28)
                .padding(.horizontal, 1.5)
                .opacity(model.tuckedEdge == nil ? 0 : 1)
                .animation(.easeOut(duration: 0.2), value: model.tuckedEdge)
        }
        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { actions.heightChanged($0) }
        .padding(PanelMetrics.margin)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
    }
}

extension PanelView {

    /// 标题栏同时是拖动区域
    private var header: some View {
        VStack(spacing: 0) {
            // 拖动把手，固定后隐藏
            Capsule()
                .fill(.secondary.opacity(model.pinned ? 0 : 0.35))
                .frame(width: 36, height: 4)
                .padding(.top, 7)

            HStack(spacing: 2) {
                Image(systemName: "doc.on.clipboard")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(Color.accentColor)
                Text("ClipMD")
                    .font(.system(size: 13, weight: .semibold))
                    .padding(.leading, 4)
                Spacer()
                ScreenshotButton(screenshot: actions.screenshot)
                IconButton(symbol: "folder", label: "打开保存文件夹", action: actions.openFolder)
                IconButton(symbol: "arrow.down.right.and.arrow.up.left", label: "贴边收起", action: actions.toggleTuck)
                IconButton(symbol: "gearshape", label: "设置", action: actions.openSettings)
            }
            .padding(.leading, 14)
            .padding(.trailing, 8)
            .padding(.top, 2)
            .padding(.bottom, 8)
        }
        .contentShape(Rectangle())
        .gesture(
            DragGesture(minimumDistance: 3)
                .onChanged { _ in actions.dragChanged() }
                .onEnded { _ in actions.dragEnded() }
        )
        .help(model.pinned ? "位置已固定，可以在菜单栏或设置里取消" : "按住这里拖动")
    }
}

/// 主按钮：把剪贴板存成 Markdown。保存成功后短暂变成绿色的"已保存"。
private struct SaveButton: View {
    let model: PanelModel
    let action: @MainActor () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                ZStack {
                    Circle().fill(.white.opacity(0.2))
                    SwapIcon(from: "square.and.arrow.down", to: "checkmark", swapped: model.justSaved)
                }
                .font(.system(size: 13, weight: .bold))
                .frame(width: 30, height: 30)

                VStack(alignment: .leading, spacing: 1) {
                    Text(model.justSaved ? "已保存为 Markdown" : "保存为 Markdown")
                        .font(.system(size: 13, weight: .semibold))
                    Text(subtitle)
                        .font(.system(size: 11))
                        .opacity(0.85)
                        .lineLimit(1)
                }
                .contentTransition(.opacity)

                Spacer(minLength: 4)

                // 保存成功时让出位置给副标题
                if !model.justSaved {
                    Text("⌃⌥S")
                        .font(.system(size: 11, weight: .semibold, design: .rounded))
                        .padding(.horizontal, 7)
                        .padding(.vertical, 3)
                        .background(.white.opacity(0.18), in: Capsule())
                        .transition(.opacity)
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 9)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
        }
        .buttonStyle(SaveButtonStyle(success: model.justSaved, dimmed: !model.hint.canSave))
        .brightness(model.freshCopy ? 0.06 : 0)
        .animation(.smooth(duration: 0.4), value: model.freshCopy)
        // 每复制一次弹一下
        .keyframeAnimator(initialValue: 1.0, trigger: model.copyPulse) { content, scale in
            content.scaleEffect(scale)
        } keyframes: { _ in
            SpringKeyframe(1.035, duration: 0.16, spring: .snappy)
            SpringKeyframe(1.0, duration: 0.4, spring: .bouncy)
        }
        .help("把剪贴板里的文字存成 .md 文件，并把文件放回剪贴板（⌃⌥S，或者连点两次复制）")
        .accessibilityLabel(model.justSaved ? "已保存为 Markdown" : "保存为 Markdown")
    }

    private var subtitle: String {
        if model.justSaved { return model.savedOnClipboard ? "文件已复制，⌘V 发送" : "单击下面第一条就能复制文件" }
        if model.freshCopy { return "刚复制了一段文字，点这里存成 md" }
        return model.hint.message
    }
}

private struct SaveButtonStyle: ButtonStyle {
    let success: Bool
    let dimmed: Bool

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .foregroundStyle(.white)
            .background {
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .fill((success ? Color.green : Color.accentColor).gradient)
                    .opacity(dimmed && !success ? 0.6 : 1)
            }
            .scaleEffect(configuration.isPressed ? 0.97 : 1)
            .animation(.spring(response: 0.25, dampingFraction: 0.7), value: configuration.isPressed)
            .animation(.smooth(duration: 0.3), value: success)
    }
}

private struct FailureBanner: View {
    let failure: Failure
    let fix: @MainActor (FailureFix) -> Void
    let dismiss: @MainActor () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            Image(systemName: "exclamationmark.circle.fill")
                .font(.system(size: 14))
                .foregroundStyle(.orange)
                .padding(.top, 1)
            VStack(alignment: .leading, spacing: 2) {
                Text(failure.title)
                    .font(.system(size: 12, weight: .semibold))
                Text(failure.detail)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                if let action = failure.fix {
                    Button(action.title) { fix(action) }
                        .buttonStyle(.link)
                        .font(.system(size: 11, weight: .medium))
                        .padding(.top, 2)
                }
            }
            Spacer(minLength: 0)
            IconButton(symbol: "xmark", label: "关闭", action: dismiss)
        }
        .padding(.leading, 10)
        .padding(.trailing, 4)
        .padding(.vertical, 8)
        .background(Color.orange.opacity(0.12), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
    }
}

/// 拖着能接住的东西经过时：上半块存成 Markdown，下半块放到剪贴板，鼠标停在哪块哪块亮。
/// 两块各占玻璃的一半，和 TrackingContainerView 判断鼠标位置的方式一致。
private struct DropZones: View {
    let active: DropAction

    var body: some View {
        VStack(spacing: 8) {
            zone(.save, symbol: "arrow.down.doc", title: "存成 Markdown", detail: "存成 .md 文件，网页格式也会保留")
            zone(.clipboard, symbol: "doc.on.clipboard", title: "放到剪贴板", detail: "不存文件，⌘V 直接粘贴")
        }
        .padding(8)
        // 垫一层磨砂，把底下的列表文字遮住（玻璃上只调透明度盖不住）
        .background(.thickMaterial, in: RoundedRectangle(cornerRadius: PanelMetrics.cornerRadius, style: .continuous))
        .animation(.snappy(duration: 0.2), value: active)
    }

    private func zone(_ action: DropAction, symbol: String, title: String, detail: String) -> some View {
        let isActive = action == active
        return ZStack {
            RoundedRectangle(cornerRadius: PanelMetrics.cornerRadius - 6, style: .continuous)
                .fill(Color.accentColor.opacity(isActive ? 0.2 : 0.05))
            RoundedRectangle(cornerRadius: PanelMetrics.cornerRadius - 6, style: .continuous)
                .strokeBorder(Color.accentColor.opacity(isActive ? 0.85 : 0.25),
                              style: StrokeStyle(lineWidth: isActive ? 2 : 1.5, dash: [7, 5]))
            HStack(spacing: 10) {
                Image(systemName: symbol)
                    .font(.system(size: 20, weight: .semibold))
                    .foregroundStyle(Color.accentColor.opacity(isActive ? 1 : 0.5))
                VStack(alignment: .leading, spacing: 2) {
                    Text(title)
                        .font(.system(size: 14, weight: .semibold))
                    Text(detail)
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                }
                .opacity(isActive ? 1 : 0.55)
            }
        }
        .frame(maxHeight: .infinity)
        .scaleEffect(isActive ? 1 : 0.97)
    }
}

/// 拖着接不住的东西经过时的灰色提示
private struct DropOverlay: View {
    let title: String
    let detail: String

    var body: some View {
        let tint = Color.secondary
        ZStack {
            RoundedRectangle(cornerRadius: PanelMetrics.cornerRadius, style: .continuous)
                .fill(.thickMaterial)
            RoundedRectangle(cornerRadius: PanelMetrics.cornerRadius, style: .continuous)
                .fill(tint.opacity(0.08))
            RoundedRectangle(cornerRadius: PanelMetrics.cornerRadius - 4, style: .continuous)
                .strokeBorder(tint.opacity(0.4), style: StrokeStyle(lineWidth: 2, dash: [7, 5]))
                .padding(4)
            VStack(spacing: 6) {
                Image(systemName: "nosign")
                    .font(.system(size: 26, weight: .semibold))
                    .foregroundStyle(tint)
                Text(title)
                    .font(.system(size: 14, weight: .semibold))
                Text(detail)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 24)
            }
        }
        .allowsHitTesting(false)
        .accessibilityElement(children: .combine)
    }
}

/// 删除后的提示，带撤销
private struct NoticeBanner: View {
    let notice: Notice
    let undo: @MainActor () -> Void
    let dismiss: @MainActor () -> Void

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "checkmark.circle.fill")
                .foregroundStyle(.secondary)
            Text(notice.message)
                .lineLimit(1)
            Spacer(minLength: 0)
            Button("撤销", action: undo)
                .buttonStyle(.link)
                .fontWeight(.medium)
            IconButton(symbol: "xmark", label: "关闭", action: dismiss)
        }
        .font(.system(size: 12))
        .padding(.leading, 10)
        .padding(.trailing, 4)
        .padding(.vertical, 4)
        .background(.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
    }
}

/// macOS 处于"每次都询问"时的常驻提示
private struct PasteTip: View {
    let fix: @MainActor (FailureFix) -> Void

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "info.circle")
                .foregroundStyle(.secondary)
            Text("macOS 每次保存都会询问是否允许粘贴")
                .foregroundStyle(.secondary)
                .lineLimit(1)
            Spacer(minLength: 0)
            Button("改成允许") { fix(.pasteSettings) }
                .buttonStyle(.link)
        }
        .font(.system(size: 11))
        .padding(.horizontal, 8)
    }
}

private struct NoteRow: View {
    let note: SavedNote
    let now: Date
    let actions: PanelActions

    @State private var hovering = false
    @State private var copied = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        HStack(spacing: 10) {
            content
                .overlay {
                    // 单击复制文件；拖进聊天窗口发送，拖到废纸篓删除；右键有更多操作
                    FileDragArea(url: note.url, help: "单击复制文件，或拖到聊天窗口里；拖到废纸篓会删除；右键有更多操作",
                                 onClick: copyFile, onDropToTrash: { actions.trash(note) }, menuItems: menuItems)
                }

            HStack(spacing: 0) {
                IconButton(symbol: "eye", label: previewLabel) { actions.preview(note) }
                IconButton(symbol: "magnifyingglass", label: "在 Finder 中显示") { actions.reveal(note) }
                DeleteMenu(note: note, actions: actions)
            }
            .opacity(hovering ? 1 : 0.45)
        }
        .padding(.horizontal, 8)
        .frame(height: PanelMetrics.rowHeight)
        .background {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(.primary.opacity(hovering ? 0.07 : 0))
        }
        .scaleEffect(hovering && !reduceMotion ? 1.01 : 1)
        .contentShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        .onHover { inside in
            withAnimation(.easeOut(duration: 0.15)) { hovering = inside }
        }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
        .accessibilityAction { copyFile() }
        .accessibilityAction(named: "预览和编辑") { actions.preview(note) }
        .accessibilityAction(named: "从列表移除") { actions.hide(note) }
        .accessibilityAction(named: "移到废纸篓") { actions.trash(note) }
    }

    /// 图标和文字；点击、拖拽、右键由盖在上面的 FileDragArea 处理
    private var content: some View {
        HStack(spacing: 10) {
            ZStack {
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .fill(copied ? Color.green.opacity(0.18) : Color.accentColor.opacity(0.14))
                SwapIcon(from: "doc.text", to: "checkmark", swapped: copied)
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(copied ? Color.green : Color.accentColor)
            }
            .frame(width: 30, height: 30)

            VStack(alignment: .leading, spacing: 2) {
                Text(note.title)
                    .font(.system(size: 13, weight: .medium))
                    .lineLimit(1)
                Text(copied ? "文件已复制，⌘V 发送" : Self.meta(for: note, now: now))
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
                    .contentTransition(.opacity)
            }

            Spacer(minLength: 4)
        }
        .contentShape(Rectangle())
    }

    private var menuItems: [FileDragArea.MenuItem] {
        [
            .init(title: previewLabel, symbol: "eye") { actions.preview(note) },
            .init(title: "复制文件", symbol: "doc.on.doc") { copyFile() },
            .init(title: "在 Finder 中显示", symbol: "folder") { actions.reveal(note) },
            .init(title: "从列表移除（保留文件）", symbol: "minus.circle", startsGroup: true) { actions.hide(note) },
            .init(title: "移到废纸篓", symbol: "trash") { actions.trash(note) },
        ]
    }

    private var previewLabel: String {
        Preferences.shared.previewAppName.map { "用 \($0) 打开" } ?? "预览和编辑"
    }

    private func copyFile() {
        guard actions.copy(note) else { return }
        withAnimation(.snappy(duration: 0.25)) { copied = true }
        Task {
            try? await Task.sleep(for: .seconds(1.4))
            withAnimation(.smooth(duration: 0.3)) { copied = false }
        }
    }

    static func meta(for note: SavedNote, now: Date) -> String {
        "\(NoteRowTime.label(for: note.date, now: now)) · \(note.characterCount.formatted()) 字"
    }
}

/// 删除按钮：两种删除都放在一个小菜单里，不会误删
private struct DeleteMenu: View {
    let note: SavedNote
    let actions: PanelActions

    @State private var hovering = false

    var body: some View {
        Menu {
            Button("从列表移除（保留文件）", systemImage: "minus.circle") { actions.hide(note) }
            Button("移到废纸篓", systemImage: "trash", role: .destructive) { actions.trash(note) }
        } label: {
            Image(systemName: "trash")
                .font(.system(size: 11, weight: .semibold))
                .frame(width: 24, height: 24)
                .background(Circle().fill(.primary.opacity(hovering ? 0.09 : 0)))
                .contentShape(Circle())
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
        .foregroundStyle(hovering ? .primary : .secondary)
        .onHover { hovering = $0 }
        .help("删除")
        .accessibilityLabel("删除")
    }
}

private struct EmptyState: View {
    var body: some View {
        VStack(spacing: 4) {
            Text("保存过的文件会出现在这里")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.secondary)
            Text("小技巧：连点两次 ⌘C 也会直接保存")
                .font(.system(size: 11))
                .foregroundStyle(.tertiary)
        }
        .frame(maxWidth: .infinity)
        .padding(.top, 8)
        .padding(.bottom, 18)
    }
}

/// 两个图标交叉缩放、淡入淡出。
/// 不用 SF Symbols 的 `.contentTransition(.symbolEffect(.replace))`：在玻璃上反复播放时，每次会临时多占约 50 MB 显存。
struct SwapIcon: View {
    let from: String
    let to: String
    let swapped: Bool

    var body: some View {
        ZStack {
            Image(systemName: from)
                .scaleEffect(swapped ? 0.4 : 1)
                .opacity(swapped ? 0 : 1)
            Image(systemName: to)
                .scaleEffect(swapped ? 1 : 0.4)
                .opacity(swapped ? 1 : 0)
        }
    }
}

struct IconButton: View {
    let symbol: String
    let label: String
    var isOn = false
    let action: @MainActor () -> Void

    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 11, weight: .semibold))
                .frame(width: 24, height: 24)
                .background(Circle().fill(isOn ? Color.accentColor.opacity(0.16) : .primary.opacity(hovering ? 0.09 : 0)))
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .foregroundStyle(isOn ? AnyShapeStyle(Color.accentColor) : AnyShapeStyle(hovering ? .primary : .secondary))
        .onHover { hovering = $0 }
        .help(label)
        .accessibilityLabel(label)
    }
}

/// "已保存 | 剪贴板"两个标签，选中的底下垫一块会滑动的高亮
private struct ListTabs: View {
    let selection: PanelTab
    let clipCount: Int
    let select: @MainActor (PanelTab) -> Void
    let clear: (@MainActor () -> Void)?

    @Namespace private var pill

    var body: some View {
        HStack(spacing: 6) {
            HStack(spacing: 2) {
                tab(.saved, title: "已保存")
                tab(.clipboard, title: clipCount > 0 ? "剪贴板 \(clipCount)" : "剪贴板")
            }
            .padding(2)
            .background(Color.primary.opacity(0.06), in: Capsule())

            Spacer()

            if let clear {
                Button("清空", action: clear)
                    .buttonStyle(.plain)
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(.secondary)
                    .help("清空剪贴板记录（可以撤销）")
                    .transition(.opacity)
            }
        }
    }

    private func tab(_ tab: PanelTab, title: String) -> some View {
        let selected = tab == selection
        return Button {
            withAnimation(.snappy(duration: 0.28)) { select(tab) }
        } label: {
            Text(title)
                .font(.system(size: 12, weight: .semibold))
                .monospacedDigit()
                .foregroundStyle(selected ? .primary : .secondary)
                .padding(.horizontal, 10)
                .padding(.vertical, 4)
                .background {
                    if selected {
                        Capsule()
                            .fill(Color(nsColor: .controlBackgroundColor).opacity(0.85))
                            .shadow(color: .black.opacity(0.08), radius: 2, y: 1)
                            .matchedGeometryEffect(id: "pill", in: pill)
                    }
                }
                .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

/// 剪贴板记录的一行：单击放回剪贴板。文字的右边是存成 md 和删除，图片的右边是识别文字和删除
private struct ClipRow: View {
    static let height = PanelMetrics.rowHeight

    let item: ClipItem
    let imageURL: URL?
    let now: Date
    let actions: PanelActions

    @State private var hovering = false
    @State private var copied = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var saved: Bool { item.savedFileName != nil }

    var body: some View {
        HStack(spacing: 10) {
            leading
                .frame(width: 30, height: 30)

            VStack(alignment: .leading, spacing: 2) {
                Text(item.title.isEmpty ? "（空白）" : item.title)
                    .font(.system(size: 13, weight: .medium))
                    .lineLimit(1)
                Text(copied ? (item.isImage ? "图片已放回剪贴板，⌘V 粘贴" : "已放回剪贴板，⌘V 粘贴") : meta)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
                    .lineLimit(1)
                    .contentTransition(.opacity)
            }

            Spacer(minLength: 4)

            HStack(spacing: 0) {
                IconButton(symbol: "eye", label: item.isImage ? "预览图片" : "预览全文") { actions.previewClip(item) }
                if item.isImage {
                    IconButton(symbol: "text.viewfinder", label: "识别图片里的文字") { actions.recognizeClip(item) }
                } else {
                    IconButton(symbol: saved ? "checkmark.circle" : "arrow.down.doc",
                               label: saved ? "已存成 md（点了再存一份）" : "存成 md", isOn: saved) { actions.saveClip(item) }
                }
                IconButton(symbol: "xmark", label: "从记录里删除") { actions.removeClip(item) }
            }
            .opacity(hovering || saved ? 1 : 0.45)
        }
        .padding(.horizontal, 8)
        .frame(height: Self.height)
        .background {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(.primary.opacity(hovering ? 0.07 : 0))
        }
        .scaleEffect(hovering && !reduceMotion ? 1.01 : 1)
        .contentShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        .onHover { inside in
            withAnimation(.easeOut(duration: 0.15)) { hovering = inside }
        }
        .onTapGesture { copyBack() }
        .contextMenu {
            Button("预览", systemImage: "eye") { actions.previewClip(item) }
            Button(item.isImage ? "放回剪贴板（图片）" : "放回剪贴板", systemImage: "doc.on.clipboard") { copyBack() }
            if item.isImage {
                Button("识别文字", systemImage: "text.viewfinder") { actions.recognizeClip(item) }
                Button("在 Finder 中显示", systemImage: "folder") { actions.revealClip(item) }
            } else {
                Button(saved ? "再存一份 md" : "存成 md", systemImage: "arrow.down.doc") { actions.saveClip(item) }
            }
            Divider()
            Button("从记录里删除", systemImage: "xmark") { actions.removeClip(item) }
        }
        .help(item.preview)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
        .accessibilityAction { copyBack() }
        .accessibilityAction(named: "预览") { actions.previewClip(item) }
        .accessibilityAction(named: "存成 md") { actions.saveClip(item) }
        .accessibilityAction(named: "从记录里删除") { actions.removeClip(item) }
    }

    /// 文字是图标，图片是缩略图；放回剪贴板时短暂变成对勾
    @ViewBuilder
    private var leading: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .fill(copied ? Color.green.opacity(0.18) : Color.primary.opacity(0.07))
            if let imageURL, !copied, let thumbnail = ClipImages.thumbnail(for: imageURL) {
                Image(nsImage: thumbnail)
                    .resizable()
                    .aspectRatio(contentMode: .fill)
                    .frame(width: 30, height: 30)
                    .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
            } else {
                SwapIcon(from: item.isImage ? "photo" : "text.alignleft", to: "checkmark", swapped: copied)
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(copied ? Color.green : Color.secondary)
            }
        }
    }

    private var meta: String {
        if item.isImage {
            var parts = [NoteRowTime.label(for: item.date, now: now)]
            if let source = item.sourceApp { parts.append(source) }
            return parts.joined(separator: " · ")
        }
        var parts = [NoteRowTime.label(for: item.date, now: now), "\(item.characterCount.formatted()) 字"]
        if let source = item.sourceApp { parts.append(source) }
        if saved { parts.append("已存") }
        return parts.joined(separator: " · ")
    }

    private func copyBack() {
        actions.copyClip(item)
        withAnimation(.snappy(duration: 0.25)) { copied = true }
        Task {
            try? await Task.sleep(for: .seconds(1.4))
            withAnimation(.smooth(duration: 0.3)) { copied = false }
        }
    }
}

private struct ClipEmptyState: View {
    let title: String
    let detail: String
    let button: String?
    let action: @MainActor () -> Void

    var body: some View {
        VStack(spacing: 4) {
            Text(title)
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            Text(detail)
                .font(.system(size: 11))
                .foregroundStyle(.tertiary)
                .multilineTextAlignment(.center)
            if let button {
                Button(button, action: action)
                    .buttonStyle(.link)
                    .font(.system(size: 11, weight: .medium))
                    .padding(.top, 2)
            }
        }
        .padding(.horizontal, 16)
        .frame(maxWidth: .infinity)
        .padding(.top, 8)
        .padding(.bottom, 18)
    }
}

/// "刚刚""3 分钟前""14:02""昨天"这类时间，已保存和剪贴板两个列表共用
enum NoteRowTime {
    static func label(for date: Date, now: Date) -> String {
        let interval = now.timeIntervalSince(date)
        if interval < 60 { return "刚刚" }
        if interval < 3600 { return "\(Int(interval / 60)) 分钟前" }
        if Calendar.current.isDate(date, inSameDayAs: now) { return date.formatted(date: .omitted, time: .shortened) }
        if Calendar.current.isDateInYesterday(date) { return "昨天" }
        return date.formatted(.dateTime.month().day())
    }
}

/// 标题栏的截图按钮：点一下截图，旁边的菜单里有"截图并识别文字"
private struct ScreenshotButton: View {
    let screenshot: @MainActor (Bool) -> Void

    @State private var hovering = false

    var body: some View {
        Menu {
            Button("截图", systemImage: "camera.viewfinder") { screenshot(false) }
            Button("截图并识别文字", systemImage: "text.viewfinder") { screenshot(true) }
        } label: {
            Image(systemName: "camera.viewfinder")
                .font(.system(size: 11, weight: .semibold))
                .frame(width: 24, height: 24)
                .background(Circle().fill(.primary.opacity(hovering ? 0.09 : 0)))
                .contentShape(Circle())
        } primaryAction: {
            screenshot(false)
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
        .foregroundStyle(hovering ? .primary : .secondary)
        .onHover { hovering = $0 }
        .help("截图（按住可以选「截图并识别文字」）")
        .accessibilityLabel("截图")
    }
}
