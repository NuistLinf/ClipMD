import Foundation

/// 剪贴板记录里的一条：复制过的文字或图片，还没存成文件
public struct ClipItem: Identifiable, Codable, Equatable, Sendable {
    public let id: UUID
    public var text: String
    /// 网页格式（从网页复制时有），存成 md 时用来转换
    public var html: String?
    public var date: Date
    /// 从哪个应用复制的
    public var sourceApp: String?
    public var characterCount: Int
    /// 列表里显示的第一行
    public var preview: String
    /// 已经存成 md 的文件名
    public var savedFileName: String?
    /// 图片：存在 images 文件夹里的文件名（文字记录为 nil）
    public var imageFile: String?
    public var imageWidth: Int?
    public var imageHeight: Int?
    /// 图片内容的指纹，用来去重
    public var imageHash: String?

    public var isImage: Bool { imageFile != nil }

    /// 列表和预览里显示的标题：第一行去掉 Markdown 标题的 # 号（旧记录存的是带 # 的第一行）
    public var title: String { ClipHistoryList.strippingHeading(preview) }

    public init(id: UUID = UUID(), text: String, html: String?, date: Date, sourceApp: String?, savedFileName: String? = nil) {
        self.id = id
        self.text = text
        self.html = html
        self.date = date
        self.sourceApp = sourceApp
        self.savedFileName = savedFileName
        characterCount = text.count
        preview = ClipHistoryList.preview(of: text)
    }

    /// 图片记录
    public init(id: UUID = UUID(), imageFile: String, width: Int, height: Int, hash: String, date: Date, sourceApp: String?) {
        self.id = id
        text = ""
        html = nil
        self.date = date
        self.sourceApp = sourceApp
        self.imageFile = imageFile
        imageWidth = width
        imageHeight = height
        imageHash = hash
        characterCount = 0
        preview = "图片 \(width) × \(height)"
    }

    /// 去重用：文字比内容，图片比指纹
    var contentKey: String { imageHash.map { "image:" + $0 } ?? "text:" + text }
}

/// 剪贴板记录的规则：去重、上限、存取。不碰剪贴板，方便测试。
public enum ClipHistoryList {
    public static let maxItems = 50
    /// 单条太大的不记（一般是整篇文档），网页格式太大的只记文字
    public static let maxTextLength = 100_000
    public static let maxHTMLLength = 300_000

    /// 空白、太长的返回 nil
    public static func makeItem(text: String, html: String?, sourceApp: String?, date: Date = .now) -> ClipItem? {
        guard text.contains(where: { !$0.isWhitespace }), text.count <= maxTextLength else { return nil }
        let html = html.flatMap { $0.count <= maxHTMLLength ? $0 : nil }
        return ClipItem(text: text, html: html, date: date, sourceApp: sourceApp)
    }

    /// 新的放最前面；同样的文字（或同一张图片）再复制一次只是挪到最前面，不重复记
    public static func adding(_ item: ClipItem, to items: [ClipItem]) -> [ClipItem] {
        let key = item.contentKey
        var list = items.filter { $0.contentKey != key }
        var item = item
        // 之前存过的话，记得它已经存成了哪个文件
        if item.savedFileName == nil { item.savedFileName = items.first { $0.contentKey == key }?.savedFileName }
        list.insert(item, at: 0)
        return Array(list.prefix(maxItems))
    }

    /// 还被记录用着的图片文件；其余的可以删掉
    public static func referencedImages(in items: [ClipItem]) -> Set<String> {
        Set(items.compactMap(\.imageFile))
    }

    static func strippingHeading(_ line: String) -> String {
        guard let range = line.range(of: #"^#{1,6}\s+"#, options: .regularExpression) else { return line }
        let stripped = line[range.upperBound...].trimmingCharacters(in: .whitespaces)
        return stripped.isEmpty ? line : stripped
    }

    static func preview(of text: String) -> String {
        let head = text.prefix(400)
        let line = head.split(whereSeparator: \.isNewline)
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .first { !$0.isEmpty } ?? ""
        return String(line.prefix(80))
    }

    public static func encode(_ items: [ClipItem]) throws -> Data {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        return try encoder.encode(items)
    }

    /// 文件坏了就当没有记录，不影响启动
    public static func decode(_ data: Data) -> [ClipItem] {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return (try? decoder.decode([ClipItem].self, from: data)) ?? []
    }
}
