import AppKit
import ClipMDCore
import os

enum Log {
    /// 查看：log show --last 10m --predicate 'subsystem == "dev.clipmd.ClipMD"'
    static let app = Logger(subsystem: "dev.clipmd.ClipMD", category: "app")
}

/// 失败时给出的"一键修复"
enum FailureFix: Equatable {
    case pasteSettings
    case filesSettings
    case screenSettings
    case chooseFolder
    case openSettings

    var title: String {
        switch self {
        case .pasteSettings, .filesSettings, .screenSettings: "打开系统设置"
        case .chooseFolder: "换个保存位置"
        case .openSettings: "打开设置"
        }
    }

    /// 系统设置里对应的页面
    var settingsURL: URL? {
        let base = "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension"
        switch self {
        case .pasteSettings: return URL(string: base + "?Privacy_Pasteboard")
        case .filesSettings: return URL(string: base + "?Privacy_FilesAndFolders")
        case .screenSettings: return URL(string: base + "?Privacy_ScreenCapture")
        case .chooseFolder, .openSettings: return nil
        }
    }
}

struct Failure: Equatable {
    let id = UUID()
    let title: String
    let detail: String
    var fix: FailureFix?
}

enum Feedback {
    /// `fromHTML`：用的是网页格式转出来的 Markdown；`onClipboard`：文件已经放到系统剪贴板（拖进来保存的不放）
    case saved(SavedNote, fromHTML: Bool, onClipboard: Bool)
    /// 放到了剪贴板、没存文件：拖进来的文字、截图、识别出的文字
    case copied(title: String, detail: String)
    /// 正在做、要等一会儿的事（识别文字）；做完用另一条提示替换掉
    case working(title: String, detail: String)
    case failed(Failure)
}
