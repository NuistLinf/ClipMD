import AppKit
import ClipMDCore
import SwiftMath
import SwiftUI

/// 用 SwiftMath 把 LaTeX 画成图片。按"公式 + 字号 + 深浅色"缓存，同一个公式只排版一次。
@MainActor
enum MathRenderer {
    struct Rendered {
        let image: NSImage
        /// 基线以下的高度，行内公式要往下移这么多才能和文字对齐
        let descent: CGFloat
    }

    private final class Entry {
        let rendered: Rendered?
        init(_ rendered: Rendered?) { self.rendered = rendered }
    }

    private static let cache: NSCache<NSString, Entry> = {
        let cache = NSCache<NSString, Entry>()
        cache.countLimit = 400
        return cache
    }()

    /// 字体还没下载、或者公式写错了，返回 nil
    static func render(_ latex: String, fontSize: CGFloat, display: Bool, dark: Bool) -> Rendered? {
        guard MathFontInstaller.shared.isReady else { return nil }
        let key = "\(display ? "D" : "T")|\(fontSize)|\(dark ? 1 : 0)|\(latex)" as NSString
        if let entry = cache.object(forKey: key) { return entry.rendered }

        let color = dark ? NSColor(white: 0.92, alpha: 1) : NSColor(white: 0.1, alpha: 1)
        var image = MathImage(latex: simplified(latex), fontSize: fontSize, textColor: color,
                              labelMode: display ? .display : .text, textAlignment: .left)
        let (error, result, layout) = image.asImage()
        let rendered = (error == nil ? result : nil).map { Rendered(image: $0, descent: layout?.descent ?? 0) }
        if rendered == nil { Log.app.notice("latex failed: \(latex, privacy: .public)") }
        cache.setObject(Entry(rendered), forKey: key)
        return rendered
    }

    /// 去掉 SwiftMath 不支持、但只影响间距的写法（维基百科的公式里很常见）。只在渲染时去，文件里的原文不动。
    /// - `\vphantom{…}`：看不见的占位符，SwiftMath 遇到会整条公式解析失败
    /// - `\\[5mu]`：换行时额外加的行距，SwiftMath 会把 `[5mu]` 当成文字画出来
    static func simplified(_ latex: String) -> String {
        latex
            .replacingOccurrences(of: #"\\vphantom\s*\{[^{}]*\}"#, with: "", options: .regularExpression)
            .replacingOccurrences(of: #"\\\\\s*\[[^\]]*\]"#, with: #"\\\\"#, options: .regularExpression)
    }
}

/// 把带行内公式占位符的文字拼成一个 Text：文字照常，公式换成图片
@MainActor
enum InlineMath {
    static func text(_ text: AttributedString, math: [MathSnippet], fontSize: CGFloat, dark: Bool) -> Text {
        var result = Text(verbatim: "")
        var cursor = text.startIndex
        while let open = text[cursor...].characters.firstIndex(of: MathExtractor.inlineOpen),
              let close = text[open...].characters.firstIndex(of: MathExtractor.inlineClose) {
            result = Text("\(result)\(Text(AttributedString(text[cursor..<open])))")
            let digits = String(text[text.characters.index(after: open)..<close].characters)
            if let index = Int(digits), math.indices.contains(index) {
                result = Text("\(result)\(piece(math[index], fontSize: fontSize, dark: dark))")
            }
            cursor = text.characters.index(after: close)
        }
        return Text("\(result)\(Text(AttributedString(text[cursor...])))")
    }

    private static func piece(_ snippet: MathSnippet, fontSize: CGFloat, dark: Bool) -> Text {
        if let rendered = MathRenderer.render(snippet.latex, fontSize: fontSize + 1, display: snippet.display, dark: dark) {
            return Text(Image(nsImage: rendered.image)).baselineOffset(-rendered.descent)
        }
        // 字体没下载或公式写错：显示原文
        let delimiter = snippet.display ? "$$" : "$"
        return Text(verbatim: "\(delimiter)\(snippet.latex)\(delimiter)")
            .font(.system(size: fontSize - 1, design: .monospaced))
            .foregroundStyle(.secondary)
    }
}

/// 独立成段的公式：居中；太宽时可以左右滚动
struct MathBlockView: View {
    let snippet: MathSnippet?

    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        let installer = MathFontInstaller.shared
        if let snippet, let rendered = MathRenderer.render(snippet.latex, fontSize: 18, display: true, dark: colorScheme == .dark) {
            ViewThatFits(in: .horizontal) {
                Image(nsImage: rendered.image)
                    .frame(maxWidth: .infinity)
                ScrollView(.horizontal, showsIndicators: false) {
                    Image(nsImage: rendered.image)
                }
            }
            .padding(.vertical, 6)
            .accessibilityLabel(snippet.latex)
        } else {
            VStack(alignment: .leading, spacing: 4) {
                ScrollView(.horizontal, showsIndicators: false) {
                    Text(verbatim: snippet?.latex ?? "")
                        .font(.system(size: 12.5, design: .monospaced))
                        .fixedSize()
                }
                if installer.isReady {
                    Text("这个公式解析不了，显示的是原文")
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                }
            }
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        }
    }
}

/// 文档里有公式、但还没下载字体时，预览顶部的提示
struct MathFontBanner: View {
    let formulaCount: Int

    var body: some View {
        let installer = MathFontInstaller.shared
        HStack(spacing: 10) {
            Image(systemName: "function")
                .font(.system(size: 15, weight: .semibold))
                .foregroundStyle(Color.accentColor)
                .frame(width: 30, height: 30)
                .background(Color.accentColor.opacity(0.12), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            VStack(alignment: .leading, spacing: 2) {
                Text("这篇文档里有 \(formulaCount) 个公式")
                    .font(.system(size: 13, weight: .semibold))
                Text(detail(installer.state))
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 8)
            switch installer.state {
            case .downloading:
                ProgressView().controlSize(.small)
            case .missing, .failed:
                Button(installer.state == .missing ? "下载公式字体" : "重试") {
                    Task { await installer.install() }
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.small)
            case .installed:
                EmptyView()
            }
        }
        .padding(12)
        .background(Color.accentColor.opacity(0.06), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(Color.accentColor.opacity(0.15)))
    }

    private func detail(_ state: MathFontInstaller.State) -> String {
        switch state {
        case .missing: "下载公式字体（\(MathFontInstaller.downloadSize)）后就能显示，现在先显示公式原文"
        case .downloading: "正在下载公式字体…"
        case .failed(let message): "下载失败：\(message)"
        case .installed: ""
        }
    }
}
