import AppKit
import SwiftUI

struct SettingsActions {
    var chooseFolder: @MainActor () -> Void
    var openFolder: @MainActor () -> Void
    var openPasteSettings: @MainActor () -> Void
    var togglePin: @MainActor () -> Void
    var moveTo: @MainActor (PanelController.Corner) -> Void
}

/// 设置窗口：只建一个，关掉后释放
@MainActor
final class SettingsWindow: NSObject, NSWindowDelegate {
    private var window: NSWindow?
    private let makeView: @MainActor () -> SettingsView

    init(makeView: @escaping @MainActor () -> SettingsView) {
        self.makeView = makeView
    }

    func show() {
        NSApp.activate()
        if let window {
            window.makeKeyAndOrderFront(nil)
            return
        }
        let hosting = NSHostingController(rootView: makeView())
        hosting.sizingOptions = [.preferredContentSize]
        let window = NSWindow(contentViewController: hosting)
        window.title = "ClipMD 设置"
        window.styleMask = [.titled, .closable, .miniaturizable]
        window.isReleasedWhenClosed = false
        window.delegate = self
        window.center()
        window.makeKeyAndOrderFront(nil)
        self.window = window
    }

    func windowWillClose(_ notification: Notification) {
        window = nil
    }
}

struct SettingsView: View {
    let store: NoteStore
    let detector: DoubleCopyDetector
    let history: ClipHistory
    let panel: PanelController
    let actions: SettingsActions

    @State private var editors: [AppChoice] = []
    @State private var pasteAccess = Pasteboard.accessBehavior
    @State private var screenAccess = Screenshot.hasPermission

    var body: some View {
        Form {
            Section("保存") {
                LabeledContent("保存位置") {
                    HStack(spacing: 8) {
                        Text(abbreviated(store.folder))
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .foregroundStyle(.secondary)
                            .help(store.folder.path(percentEncoded: false))
                        Button("更改…", action: actions.chooseFolder)
                        Button("打开", action: actions.openFolder)
                    }
                }
                Toggle(isOn: Binding(get: { Preferences.shared.convertHTML }, set: { Preferences.shared.setConvertHTML($0) })) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("网页内容转成 Markdown")
                        Text("在网页上选中复制时，保留标题、列表、代码块、表格和公式。AI 复制按钮给的本来就是 Markdown，不受影响")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }

            Section {
                ForEach(HotKeyAction.allCases, id: \.self) { action in
                    LabeledContent(action.title) { ShortcutRecorder(action: action) }
                }
            } header: {
                Text("快捷键")
            } footer: {
                Text("点一下再按新的组合键，至少要按住 ⌘、⌃、⌥ 中的一个；按 Esc 取消。")
                    .foregroundStyle(.secondary)
            }

            Section {
                Picker("点眼睛按钮时", selection: previewAppBinding) {
                    Text("ClipMD 内置预览").tag(URL?.none)
                    if !pickerApps.isEmpty { Divider() }
                    ForEach(pickerApps) { app in
                        Label { Text(app.name) } icon: { Image(nsImage: app.icon) }
                            .tag(Optional(app.url))
                    }
                    Divider()
                    Text("选择其他应用…").tag(Optional(Self.otherAppTag))
                }
                mathFontRow
            } header: {
                Text("预览")
            } footer: {
                Text("内置预览可以切换预览、编辑、并排，自动保存。选了别的应用后，眼睛按钮会直接用它打开文件。")
                    .foregroundStyle(.secondary)
            }

            Section {
                Toggle("监听复制", isOn: Binding(get: { detector.isEnabled }, set: { detector.setEnabled($0) }))
                Toggle(isOn: Binding(get: { history.isEnabled }, set: { history.setEnabled($0) })) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("记录剪贴板")
                        Text("复制过的文字先记在悬浮窗的「剪贴板」里，不存成文件，需要时再存成 md。只记文字，最多 50 条，保存在本机；密码管理器复制的内容不记")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                .disabled(!detector.isEnabled)
                LabeledContent("已记录 \(history.items.count) 条") {
                    Button("清空记录") { history.clear() }
                        .disabled(history.items.isEmpty)
                }
            } header: {
                Text("复制")
            } footer: {
                Text("同一段文字 3 秒内复制两次会自动保存；每次复制文字后，悬浮窗里的保存按钮会提示。关掉「监听复制」后完全不再查看剪贴板，剪贴板记录也会停止。")
                    .foregroundStyle(.secondary)
            }

            Section("悬浮窗") {
                Toggle("固定位置", isOn: Binding(get: { panel.isPinned }, set: { _ in actions.togglePin() }))
                LabeledContent("移到") {
                    HStack(spacing: 6) {
                        ForEach(PanelController.Corner.allCases, id: \.self) { corner in
                            Button(corner.title) { actions.moveTo(corner) }
                        }
                    }
                }
            }

            Section {
                LabeledContent("读取剪贴板") {
                    HStack(spacing: 8) {
                        Text(pasteAccessText).foregroundStyle(.secondary)
                        Button("打开系统设置", action: actions.openPasteSettings)
                    }
                }
                LabeledContent("录制屏幕") {
                    HStack(spacing: 8) {
                        Text(screenAccess ? "已允许" : "未允许").foregroundStyle(.secondary)
                        Button("打开系统设置") {
                            if let url = FailureFix.screenSettings.settingsURL { NSWorkspace.shared.open(url) }
                        }
                    }
                }
            } header: {
                Text("权限")
            } footer: {
                Text("读取剪贴板建议设成「允许」，否则每次保存 macOS 都会询问。录制屏幕是截图要用的，打开后要重新启动 ClipMD 才生效。")
                    .foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .frame(width: 540)
        .fixedSize(horizontal: false, vertical: true)
        .onAppear {
            editors = MarkdownApps.list()
            pasteAccess = Pasteboard.accessBehavior
            screenAccess = Screenshot.hasPermission
        }
        // 从系统设置改完权限回来时刷新
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            pasteAccess = Pasteboard.accessBehavior
            screenAccess = Screenshot.hasPermission
        }
    }

    private static let otherAppTag = URL(string: "clipmd://other-app")!

    /// 列表里的编辑器；之前选过的应用不在列表里时也要显示出来
    private var pickerApps: [AppChoice] {
        guard let current = Preferences.shared.previewApp, !editors.contains(where: { $0.url == current }) else { return editors }
        return editors + [MarkdownApps.choice(for: current)]
    }

    private var previewAppBinding: Binding<URL?> {
        Binding(get: { Preferences.shared.previewApp }, set: { choice in
            if choice == Self.otherAppTag {
                if let app = MarkdownApps.pickOtherApp() { Preferences.shared.setPreviewApp(app) }
            } else {
                Preferences.shared.setPreviewApp(choice)
            }
        })
    }

    @ViewBuilder
    private var mathFontRow: some View {
        let installer = MathFontInstaller.shared
        LabeledContent {
            switch installer.state {
            case .installed:
                Button("移除") { installer.remove() }
            case .downloading:
                ProgressView().controlSize(.small)
            case .missing, .failed:
                Button("下载") { Task { await installer.install() } }
            }
        } label: {
            VStack(alignment: .leading, spacing: 2) {
                Text("公式字体")
                Text(fontStatus(installer.state))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func fontStatus(_ state: MathFontInstaller.State) -> String {
        switch state {
        case .installed: "已下载，预览里的 LaTeX 公式会渲染出来"
        case .downloading: "正在下载…"
        case .missing: "没下载，公式显示原文（下载\(MathFontInstaller.downloadSize)）"
        case .failed(let message): "下载失败：\(message)"
        }
    }

    private var pasteAccessText: String {
        switch pasteAccess {
        case .alwaysAllow: "始终允许"
        case .alwaysDeny: "已拒绝"
        case .ask: "每次询问"
        default: "还没问过"
        }
    }

    private func abbreviated(_ url: URL) -> String {
        let path = url.path(percentEncoded: false)
        let home = FileManager.default.homeDirectoryForCurrentUser.path(percentEncoded: false)
        return path.hasPrefix(home) ? "~/" + path.dropFirst(home.count) : path
    }
}

/// 录快捷键：点一下进入录制，按下新的组合键就生效
private struct ShortcutRecorder: View {
    let action: HotKeyAction

    @State private var recording = false
    @State private var message: String?
    @State private var monitor: Any?

    var body: some View {
        let center = HotKeyCenter.shared
        let current = center.shortcuts[action] ?? action.defaultShortcut
        HStack(spacing: 8) {
            if let message {
                Text(message)
                    .font(.caption)
                    .foregroundStyle(.orange)
                    .lineLimit(1)
                    .transition(.opacity)
            }
            Button {
                recording ? stop() : start()
            } label: {
                Text(recording ? "按下新的快捷键…" : current.display)
                    .font(.system(.body, design: .rounded).weight(.medium))
                    .frame(minWidth: 110)
            }
            .buttonStyle(.bordered)
            .tint(recording ? .accentColor : nil)
            if current != action.defaultShortcut, !recording {
                Button("恢复默认") { apply(action.defaultShortcut) }
                    .buttonStyle(.link)
                    .font(.caption)
            }
        }
        .animation(.snappy(duration: 0.2), value: recording)
        .animation(.snappy(duration: 0.2), value: message)
        .onDisappear { stop() }
    }

    private func start() {
        message = nil
        recording = true
        HotKeyCenter.shared.pauseAll()
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            if event.keyCode == 53 { // Esc
                stop()
                return nil
            }
            guard let shortcut = Shortcut(event: event) else {
                message = "至少要按住 ⌘、⌃、⌥ 中的一个"
                return nil
            }
            stop()
            apply(shortcut)
            return nil
        }
    }

    private func stop() {
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
        if recording { HotKeyCenter.shared.resumeAll() }
        recording = false
    }

    private func apply(_ shortcut: Shortcut) {
        let center = HotKeyCenter.shared
        // 和另一个操作重复了
        if let other = HotKeyAction.allCases.first(where: { $0 != action && center.shortcuts[$0] == shortcut }) {
            message = "和「\(other.title)」重复了"
            return
        }
        message = center.update(action, to: shortcut) ? nil : "\(shortcut.display) 被别的应用占用了"
    }
}
