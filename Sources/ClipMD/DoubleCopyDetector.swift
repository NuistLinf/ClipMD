import AppKit
import Observation

/// 监听复制：检测"连点两次复制"，并在复制了一段文字时通知界面。
///
/// macOS 没有剪贴板变化通知，只能轮询 `changeCount`。这只是读一个计数，不读内容，
/// 不会触发系统的粘贴询问。要分辨间隔很近的两次复制，需要每 0.1 秒看一次；
/// 所以做成可以关掉的功能，屏幕休眠时也会暂停。
///
/// 判断规则：
/// - 系统允许 ClipMD 随时读剪贴板（"始终允许"）时：同一段文字 3 秒内被复制两次才算。
///   时间放宽是为了照顾复制后会卡一下、或者按钮要恢复一下的网页；要求内容相同，所以不容易误触。
///   内容只算一个哈希留在内存里，不会存下来。
/// - 否则只能看计数：0.6 秒内变了两次就算。
///
/// 同一个应用一次复制让计数一下子加 2 的，落在同一次轮询里，只算一次变化。
@MainActor
@Observable
final class DoubleCopyDetector {
    static let enabledKey = "doubleCopyEnabled"
    private static let pollInterval: TimeInterval = 0.1
    private static let countWindow: Duration = .milliseconds(600)
    private static let sameTextWindow: Duration = .seconds(3)

    @ObservationIgnored var onDoubleCopy: (@MainActor () -> Void)?
    /// 复制了一段文字（不算连点的第二下）
    @ObservationIgnored var onTextCopied: (@MainActor () -> Void)?
    /// 读到了复制的文字：文字、网页格式、来源应用。给剪贴板记录用；只在系统"始终允许"时才会读
    @ObservationIgnored var onTextRead: (@MainActor (String, String?, String?) -> Void)?
    /// 复制了一张图片：图片数据、来源应用。只在系统"始终允许"时才会读
    @ObservationIgnored var onImageRead: (@MainActor (Data, String?) -> Void)?

    private struct Change {
        let at: ContinuousClock.Instant
        let textHash: Int?
    }

    /// 看到了变化，但剪贴板还是空的：应用"先清空、再写入"，正好在两步中间被我们看到
    private struct Pending {
        let at: ContinuousClock.Instant
        let delta: Int
        var retries: Int
    }

    private static let maxEmptyRetries = 5

    @ObservationIgnored private var timer: Timer?
    @ObservationIgnored private var lastCount = 0
    @ObservationIgnored private var lastChange: Change?
    @ObservationIgnored private var pending: Pending?
    @ObservationIgnored private var observers: [NSObjectProtocol] = []

    /// 设置窗口和菜单栏里的开关
    private(set) var isEnabled = UserDefaults.standard.object(forKey: DoubleCopyDetector.enabledKey) as? Bool ?? true

    func setEnabled(_ enabled: Bool) {
        isEnabled = enabled
        UserDefaults.standard.set(enabled, forKey: Self.enabledKey)
        enabled ? start() : stop()
    }

    init() {
        // ClipMD 自己写剪贴板不算用户的复制
        Pasteboard.onOwnWrite = { [weak self] in self?.syncWithPasteboard() }

        let center = NSWorkspace.shared.notificationCenter
        observers = [
            center.addObserver(forName: NSWorkspace.screensDidSleepNotification, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.stop() }
            },
            center.addObserver(forName: NSWorkspace.screensDidWakeNotification, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { if self?.isEnabled == true { self?.start() } }
            },
        ]
    }

    func startIfEnabled() {
        if isEnabled { start() }
    }

    private func start() {
        guard timer == nil else { return }
        syncWithPasteboard()
        let timer = Timer(timeInterval: Self.pollInterval, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.tick() }
        }
        // 允许系统把唤醒和别的事件合并，省电
        timer.tolerance = 0.03
        RunLoop.main.add(timer, forMode: .common)
        self.timer = timer
        Log.app.notice("copy watcher started")
    }

    private func stop() {
        timer?.invalidate()
        timer = nil
        lastChange = nil
        pending = nil
    }

    private func syncWithPasteboard() {
        lastCount = Pasteboard.changeCount
        lastChange = nil
        pending = nil
    }

    private func tick() {
        let count = Pasteboard.changeCount
        if count != lastCount {
            let delta = count - lastCount
            lastCount = count
            evaluate(Pending(at: .now, delta: delta, retries: 0))
        } else if let pending {
            // 内容写进来时计数不会再变，所以要主动再看一次
            evaluate(pending)
        }
    }

    private func evaluate(_ change: Pending) {
        let hint = Pasteboard.hint()
        if hint == .empty, change.retries < Self.maxEmptyRetries {
            pending = Pending(at: change.at, delta: change.delta, retries: change.retries + 1)
            return
        }
        pending = nil

        let now = change.at
        let delta = change.delta
        // 只有"始终允许"时才看内容，否则每次复制都会弹系统询问
        let text = hint.canSave && Pasteboard.accessBehavior == .alwaysAllow ? Pasteboard.peekText() : nil
        let hash = text?.hashValue
        let previous = lastChange
        let isDouble = previous.map { Self.isDoubleCopy(previous: $0, now: now, hash: hash) } ?? false

        let gap = previous.map { Int((now - $0.at) / .milliseconds(1)) } ?? -1
        let app = NSWorkspace.shared.frontmostApplication?.bundleIdentifier ?? "?"
        let sameText = hash != nil && hash == previous?.textHash
        Log.app.notice("clipboard +\(delta) gap=\(gap)ms app=\(app, privacy: .public) kind=\(hint.logName, privacy: .public) sameText=\(sameText) double=\(isDouble)")

        if let text {
            onTextRead?(text, Pasteboard.html(), NSWorkspace.shared.frontmostApplication?.localizedName)
        } else if hint == .image, Pasteboard.accessBehavior == .alwaysAllow, let data = Pasteboard.imageData() {
            onImageRead?(data, NSWorkspace.shared.frontmostApplication?.localizedName)
        }

        if isDouble {
            lastChange = nil
            onDoubleCopy?()
        } else {
            lastChange = Change(at: now, textHash: hash)
            if hint.canSave { onTextCopied?() }
        }
    }

    private static func isDoubleCopy(previous: Change, now: ContinuousClock.Instant, hash: Int?) -> Bool {
        let gap = now - previous.at
        if let hash, let previousHash = previous.textHash {
            return hash == previousHash && gap <= sameTextWindow
        }
        return gap <= countWindow
    }
}
