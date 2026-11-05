import AppKit
import Carbon.HIToolbox

/// 一个快捷键：Carbon 的键码和修饰键，加上显示用的文字（"⌃⌥S"）
struct Shortcut: Codable, Equatable {
    var keyCode: UInt32
    var modifiers: UInt32
    var display: String

    /// 从按键事件生成；至少要有 ⌘、⌃、⌥ 之一，单按 ⇧ 或不按修饰键不行
    init?(event: NSEvent) {
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        guard !flags.intersection([.command, .control, .option]).isEmpty else { return nil }
        var carbon: UInt32 = 0
        var symbols = ""
        if flags.contains(.control) { carbon |= UInt32(controlKey); symbols += "⌃" }
        if flags.contains(.option) { carbon |= UInt32(optionKey); symbols += "⌥" }
        if flags.contains(.shift) { carbon |= UInt32(shiftKey); symbols += "⇧" }
        if flags.contains(.command) { carbon |= UInt32(cmdKey); symbols += "⌘" }
        keyCode = UInt32(event.keyCode)
        modifiers = carbon
        display = symbols + Self.keyName(keyCode: Int(event.keyCode), characters: event.charactersIgnoringModifiers)
    }

    init(keyCode: Int, modifiers: Int, display: String) {
        self.keyCode = UInt32(keyCode)
        self.modifiers = UInt32(modifiers)
        self.display = display
    }

    private static let specialKeys: [Int: String] = [
        kVK_Space: "空格", kVK_Return: "↩", kVK_Tab: "⇥", kVK_Delete: "⌫", kVK_ForwardDelete: "⌦",
        kVK_LeftArrow: "←", kVK_RightArrow: "→", kVK_UpArrow: "↑", kVK_DownArrow: "↓",
        kVK_F1: "F1", kVK_F2: "F2", kVK_F3: "F3", kVK_F4: "F4", kVK_F5: "F5", kVK_F6: "F6",
        kVK_F7: "F7", kVK_F8: "F8", kVK_F9: "F9", kVK_F10: "F10", kVK_F11: "F11", kVK_F12: "F12",
    ]

    private static func keyName(keyCode: Int, characters: String?) -> String {
        if let name = specialKeys[keyCode] { return name }
        return (characters ?? "?").uppercased()
    }
}

/// 可以设置快捷键的操作
enum HotKeyAction: UInt32, CaseIterable {
    case save = 1
    case screenshot = 2
    case screenshotOCR = 3

    var title: String {
        switch self {
        case .save: "保存剪贴板为 Markdown"
        case .screenshot: "截图"
        case .screenshotOCR: "截图并识别文字"
        }
    }

    var defaultShortcut: Shortcut {
        switch self {
        case .save: Shortcut(keyCode: kVK_ANSI_S, modifiers: controlKey | optionKey, display: "⌃⌥S")
        case .screenshot: Shortcut(keyCode: kVK_ANSI_A, modifiers: controlKey | optionKey, display: "⌃⌥A")
        case .screenshotOCR: Shortcut(keyCode: kVK_ANSI_O, modifiers: controlKey | optionKey, display: "⌃⌥O")
        }
    }

    fileprivate var defaultsKey: String { "shortcut.\(rawValue)" }
}

/// 用 Carbon 注册全局快捷键。按键由系统直接派发给我们，不需要监听键盘，也不需要辅助功能权限。
@MainActor
@Observable
final class HotKeyCenter {
    static let shared = HotKeyCenter()

    /// 当前每个操作的快捷键（设置界面显示用）
    private(set) var shortcuts: [HotKeyAction: Shortcut] = [:]
    @ObservationIgnored var onPress: (@MainActor (HotKeyAction) -> Void)?

    @ObservationIgnored private var refs: [HotKeyAction: EventHotKeyRef] = [:]
    @ObservationIgnored private var handlerRef: EventHandlerRef?

    private init() {
        for action in HotKeyAction.allCases {
            shortcuts[action] = UserDefaults.standard.data(forKey: action.defaultsKey)
                .flatMap { try? JSONDecoder().decode(Shortcut.self, from: $0) } ?? action.defaultShortcut
        }
    }

    /// 启动时注册全部；返回注册失败的操作
    @discardableResult
    func registerAll() -> [HotKeyAction] {
        installHandler()
        return HotKeyAction.allCases.filter { action in
            guard let shortcut = shortcuts[action] else { return false }
            return !register(action, shortcut)
        }
    }

    /// 换一个快捷键。注册失败（比如被别的应用占了）时保留原来的，返回 false
    func update(_ action: HotKeyAction, to shortcut: Shortcut) -> Bool {
        let old = shortcuts[action]
        unregister(action)
        guard register(action, shortcut) else {
            if let old { _ = register(action, old) }
            return false
        }
        shortcuts[action] = shortcut
        if let data = try? JSONEncoder().encode(shortcut) {
            UserDefaults.standard.set(data, forKey: action.defaultsKey)
        }
        return true
    }

    /// 录新快捷键时先全部停掉，不然按下的组合键会被已有的快捷键截走
    func pauseAll() {
        HotKeyAction.allCases.forEach(unregister)
    }

    func resumeAll() {
        for action in HotKeyAction.allCases {
            if let shortcut = shortcuts[action] { _ = register(action, shortcut) }
        }
    }

    private func register(_ action: HotKeyAction, _ shortcut: Shortcut) -> Bool {
        var ref: EventHotKeyRef?
        let id = EventHotKeyID(signature: OSType(0x434C_4D44), id: action.rawValue) // "CLMD"
        let status = RegisterEventHotKey(shortcut.keyCode, shortcut.modifiers, id, GetEventDispatcherTarget(), 0, &ref)
        Log.app.notice("hotkey \(action.rawValue) \(shortcut.display, privacy: .public) register=\(status)")
        guard status == noErr, let ref else { return false }
        refs[action] = ref
        return true
    }

    private func unregister(_ action: HotKeyAction) {
        if let ref = refs.removeValue(forKey: action) { UnregisterEventHotKey(ref) }
    }

    private func installHandler() {
        guard handlerRef == nil else { return }
        var eventType = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        // 用 event dispatcher target：应用不在前台（菜单栏应用一直如此）时也能收到
        InstallEventHandler(GetEventDispatcherTarget(), { _, event, _ in
            var id = EventHotKeyID()
            let status = GetEventParameter(event, EventParamName(kEventParamDirectObject), EventParamType(typeEventHotKeyID),
                                           nil, MemoryLayout<EventHotKeyID>.size, nil, &id)
            guard status == noErr, id.signature == OSType(0x434C_4D44), let action = HotKeyAction(rawValue: id.id) else {
                return OSStatus(eventNotHandledErr)
            }
            // Carbon 事件在主线程的 run loop 上派发
            MainActor.assumeIsolated { HotKeyCenter.shared.onPress?(action) }
            return noErr
        }, 1, &eventType, nil, &handlerRef)
    }
}
