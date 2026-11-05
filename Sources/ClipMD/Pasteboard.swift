import AppKit

enum ClipboardContent {
    case text(String)
    case image
    case empty
    case file
    case concealed
    /// macOS 没有允许读取（用户在"允许粘贴"的询问里拒绝了，或在系统设置里设成了拒绝）
    case denied

    var isSavable: Bool {
        if case .text = self { return true }
        return false
    }

    var isDenied: Bool {
        if case .denied = self { return true }
        return false
    }

    var logDescription: String {
        switch self {
        case .text(let text): "text(\(text.count) chars)"
        case .image: "image"
        case .empty: "empty"
        case .file: "file"
        case .concealed: "concealed"
        case .denied: "denied"
        }
    }
}

/// 剪贴板里大概是什么。只看类型、不读内容，不会触发系统的"允许粘贴"询问。
enum ClipboardHint: Equatable {
    case text(fromWeb: Bool)
    case image
    case file
    case empty
    case concealed

    var canSave: Bool {
        if case .text = self { return true }
        return false
    }

    var logName: String {
        switch self {
        case .text(fromWeb: true): "web-text"
        case .text(fromWeb: false): "text"
        case .image: "image"
        case .file: "file"
        case .empty: "empty"
        case .concealed: "concealed"
        }
    }

    @MainActor var message: String {
        switch self {
        case .text(fromWeb: true): Preferences.shared.convertHTML ? "网页文字，会保留标题、列表等格式" : "剪贴板里有网页上复制的文字"
        case .text(fromWeb: false): "剪贴板里有文字，点一下就存"
        case .image: "剪贴板里是图片，可以在「剪贴板」里识别文字"
        case .file: "剪贴板里是文件，先复制一段文字"
        case .empty: "先复制一段文字"
        case .concealed: "剪贴板里是密码类内容，不会保存"
        }
    }
}

@MainActor
enum Pasteboard {
    /// 测试时可以用启动参数 `-pasteboardName <名字>` 换成独立的剪贴板，不影响系统剪贴板
    static let board: NSPasteboard = UserDefaults.standard.string(forKey: "pasteboardName")
        .map { NSPasteboard(name: NSPasteboard.Name($0)) } ?? .general

    // nspasteboard.org 的约定：密码管理器复制的敏感内容会带这两个标记
    private static let concealedType = NSPasteboard.PasteboardType("org.nspasteboard.ConcealedType")
    private static let transientType = NSPasteboard.PasteboardType("org.nspasteboard.TransientType")

    /// ClipMD 自己写完剪贴板后调用，让连点检测忽略这次变化
    static var onOwnWrite: (@MainActor () -> Void)?

    static var changeCount: Int { board.changeCount }
    static var accessBehavior: NSPasteboard.AccessBehavior { board.accessBehavior }

    /// 只在用户触发保存时读一次，平时不碰内容
    static func read() -> ClipboardContent {
        switch hint() {
        case .concealed: return .concealed
        case .file: return .file
        case .image: return .image
        case .empty: return .empty
        case .text: break
        }
        if board.accessBehavior == .alwaysDeny { return .denied }
        // macOS 15.4 起，系统会在这里询问"是否允许粘贴"；被拒绝时拿到 nil
        guard let text = board.string(forType: .string) else { return .denied }
        guard text.contains(where: { !$0.isWhitespace }) else { return .empty }
        return .text(text)
    }

    /// 网页格式（浏览器复制时会带上），用来转成 Markdown
    static func html() -> String? {
        guard board.types?.contains(.html) == true else { return nil }
        return board.string(forType: .html)
    }

    /// 给连点检测和剪贴板记录用。调用前要确认系统是"始终允许"，否则会弹询问。
    static func peekText() -> String? {
        board.string(forType: .string)
    }

    static func hint() -> ClipboardHint {
        let types = board.types ?? []
        if types.contains(concealedType) || types.contains(transientType) { return .concealed }
        if types.contains(.fileURL) { return .file }
        if types.contains(.string) { return .text(fromWeb: types.contains(.html)) }
        if types.contains(where: imageTypes.contains) { return .image }
        return .empty
    }

    /// 和在 Finder 里对文件按 ⌘C 效果相同，粘贴到微信、邮件里就是附件
    static func writeFile(_ url: URL) {
        board.clearContents()
        board.writeObjects([url as NSURL])
        onOwnWrite?()
    }

    /// 文字和网页格式一起放：粘贴到支持格式的应用里能保留标题、列表这些
    static func writeText(_ text: String, html: String?) {
        board.clearContents()
        board.setString(text, forType: .string)
        if let html { board.setString(html, forType: .html) }
        onOwnWrite?()
    }

    private static let imageTypes: [NSPasteboard.PasteboardType] = [.png, .tiff, .init("public.jpeg"), .init("public.heic")]

    /// 剪贴板里的图片数据（给剪贴板记录用，调用前要确认系统是"始终允许"）
    static func imageData() -> Data? {
        for type in imageTypes {
            if let data = board.data(forType: type) { return data }
        }
        return nil
    }

    /// 放一张图片：PNG 直接给；有的应用只认 TIFF，等它要的时候再转，不预先占内存
    static func writeImage(_ png: Data) {
        let item = NSPasteboardItem()
        item.setData(png, forType: .png)
        item.setDataProvider(TIFFProvider(png: png), forTypes: [.tiff])
        board.clearContents()
        board.writeObjects([item])
        onOwnWrite?()
    }

    static func writeString(_ string: String) {
        board.clearContents()
        board.setString(string, forType: .string)
        onOwnWrite?()
    }
}

/// 有应用来要 TIFF 时才从 PNG 转
private final class TIFFProvider: NSObject, NSPasteboardItemDataProvider {
    private let png: Data

    init(png: Data) { self.png = png }

    func pasteboard(_ pasteboard: NSPasteboard?, item: NSPasteboardItem, provideDataForType type: NSPasteboard.PasteboardType) {
        guard type == .tiff, let tiff = NSBitmapImageRep(data: png)?.tiffRepresentation else { return }
        item.setData(tiff, forType: .tiff)
    }
}
