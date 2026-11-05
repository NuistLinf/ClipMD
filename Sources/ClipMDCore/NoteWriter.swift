import Foundation

public struct SavedNote: Identifiable, Hashable, Sendable {
    public let url: URL
    public let title: String
    public let date: Date
    public let characterCount: Int

    public var id: URL { url }

    public init(url: URL, title: String, date: Date, characterCount: Int) {
        self.url = url
        self.title = title
        self.date = date
        self.characterCount = characterCount
    }
}

/// 文件读写。全部是同步调用，由调用方放到后台执行。
public enum NoteWriter {
    public static func save(text: String, to folder: URL, date: Date = .now) throws -> SavedNote {
        let fm = FileManager.default
        try fm.createDirectory(at: folder, withIntermediateDirectories: true)

        let title = NoteNamer.title(for: text)
        var url = folder.appending(path: NoteNamer.fileName(title: title, date: date))
        var suffix = 2
        while fm.fileExists(atPath: url.path(percentEncoded: false)) {
            url = folder.appending(path: NoteNamer.fileName(title: title, date: date, suffix: suffix))
            suffix += 1
        }

        let body = text.hasSuffix("\n") ? text : text + "\n"
        try body.write(to: url, atomically: true, encoding: .utf8)
        return SavedNote(url: url, title: title, date: date, characterCount: text.count)
    }

    /// 编辑后保存。保留原来的创建时间，免得改过的文件在列表里跳到最前面。
    public static func overwrite(text: String, at url: URL) throws {
        let fm = FileManager.default
        let created = (try? fm.attributesOfItem(atPath: url.path(percentEncoded: false)))?[.creationDate] as? Date
        try text.write(to: url, atomically: true, encoding: .utf8)
        if let created {
            try? fm.setAttributes([.creationDate: created], ofItemAtPath: url.path(percentEncoded: false))
        }
    }

    /// 最近保存的几条，按创建时间倒序。只读取最终要显示的那几个文件的内容来数字数。
    /// `hidden` 是从列表里移除、但文件还在的文件名。
    public static func recentNotes(in folder: URL, limit: Int, hidden: Set<String> = []) -> [SavedNote] {
        let keys: [URLResourceKey] = [.creationDateKey, .isRegularFileKey]
        guard let urls = try? FileManager.default.contentsOfDirectory(
            at: folder, includingPropertiesForKeys: keys, options: [.skipsHiddenFiles]
        ) else { return [] }

        let dated: [(URL, Date)] = urls.compactMap { url in
            guard url.pathExtension.lowercased() == "md",
                  !hidden.contains(url.lastPathComponent),
                  let values = try? url.resourceValues(forKeys: Set(keys)),
                  values.isRegularFile == true else { return nil }
            return (url, values.creationDate ?? .distantPast)
        }

        return dated
            .sorted { $0.1 > $1.1 }
            .prefix(limit)
            .map { url, date in
                let count = (try? String(contentsOf: url, encoding: .utf8))?
                    .trimmingCharacters(in: .newlines).count ?? 0
                return SavedNote(url: url, title: displayTitle(fromFileName: url.deletingPathExtension().lastPathComponent),
                                 date: date, characterCount: count)
            }
    }

    /// 去掉文件名里的 `2026-10-24 ` 日期前缀
    static func displayTitle(fromFileName name: String) -> String {
        guard let range = name.range(of: #"^\d{4}-\d{2}-\d{2} "#, options: .regularExpression) else { return name }
        return String(name[range.upperBound...])
    }
}
