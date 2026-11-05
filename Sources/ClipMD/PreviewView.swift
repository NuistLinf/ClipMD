import AppKit
import ClipMDCore
import SwiftUI

struct PreviewActions {
    var copyFile: @MainActor () -> Void
    var reveal: @MainActor () -> Void
    var apps: [AppChoice]
    var openWith: @MainActor (URL) -> Void
}

struct PreviewView: View {
    let document: NoteDocument
    let actions: PreviewActions

    @State private var copied = false

    var body: some View {
        content
            .frame(minWidth: 480, minHeight: 360)
            .navigationTitle(document.title)
            .navigationSubtitle(document.statusText)
            .toolbar {
                ToolbarItem(placement: .principal) {
                    Picker("显示方式", selection: Binding(get: { document.mode }, set: { document.setMode($0) })) {
                        ForEach(NoteDocument.Mode.allCases) { Text($0.title).tag($0) }
                    }
                    .pickerStyle(.segmented)
                    .help("预览 ⌘1 · 编辑 ⌘2 · 并排 ⌘3")
                }
                ToolbarItemGroup(placement: .primaryAction) {
                    Button {
                        actions.copyFile()
                        copied = true
                        Task {
                            try? await Task.sleep(for: .seconds(1.5))
                            copied = false
                        }
                    } label: {
                        Label(copied ? "已复制" : "复制文件", systemImage: copied ? "checkmark" : "doc.on.doc")
                    }
                    .help("把文件放到剪贴板，⌘V 发送")

                    Button(action: actions.reveal) {
                        Label("在 Finder 中显示", systemImage: "folder")
                    }
                    .help("在 Finder 中显示")

                    Menu {
                        ForEach(actions.apps) { app in
                            Button { actions.openWith(app.url) } label: {
                                Label { Text(app.name) } icon: { Image(nsImage: app.icon) }
                            }
                        }
                        Divider()
                        Button("其他应用…") {
                            if let app = MarkdownApps.pickOtherApp() { actions.openWith(app) }
                        }
                    } label: {
                        Label("打开方式", systemImage: "arrow.up.forward.app")
                    }
                    .help("用其他应用打开")
                }
            }
    }

    @ViewBuilder
    private var content: some View {
        if !document.isLoaded {
            ProgressView()
                .controlSize(.small)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            switch document.mode {
            case .preview:
                MarkdownPreview(document: document.parsed)
            case .edit:
                editor
            case .split:
                HSplitView {
                    editor.frame(minWidth: 240, maxWidth: .infinity)
                    MarkdownPreview(document: document.parsed).frame(minWidth: 240, maxWidth: .infinity)
                }
            }
        }
    }

    private var editor: some View {
        MarkdownTextView(text: Binding(get: { document.text }, set: { document.update(text: $0) }))
    }
}

// MARK: - 渲染

struct MarkdownPreview: View {
    let document: MarkdownDocument

    var body: some View {
        ScrollView {
            MarkdownContent(document: document, lazy: true)
                .frame(maxWidth: 720, alignment: .leading)
                .padding(.horizontal, 36)
                .padding(.vertical, 28)
                .frame(maxWidth: .infinity)
                .textSelection(.enabled)
        }
    }
}

/// 排好版的内容（不带滚动）。`lazy` 为 false 时一次排完，测试截图用。
struct MarkdownContent: View {
    let document: MarkdownDocument
    var lazy = true

    var body: some View {
        if lazy {
            LazyVStack(alignment: .leading, spacing: 0) { rows }
        } else {
            VStack(alignment: .leading, spacing: 0) { rows }
        }
    }

    @ViewBuilder
    private var rows: some View {
        let blocks = document.blocks
        if !document.math.isEmpty, !MathFontInstaller.shared.isReady {
            MathFontBanner(formulaCount: document.math.count)
                .padding(.bottom, 22)
        }
        ForEach(Array(blocks.enumerated()), id: \.element.id) { index, block in
            BlockView(block: block, math: document.math)
                .padding(.top, index == 0 ? 0 : MarkdownPreview.spacing(before: block, after: blocks[index - 1]))
        }
    }
}

extension MarkdownPreview {
    static func spacing(before block: MarkdownBlock, after previous: MarkdownBlock) -> CGFloat {
        switch block.kind {
        case .heading(let level): return level <= 2 ? 24 : 18
        case .thematicBreak: return 12
        default: break
        }
        if block.listDepth > 0, previous.listDepth > 0 { return 5 }
        if case .heading = previous.kind { return 10 }
        return 12
    }
}

private struct BlockView: View {
    let block: MarkdownBlock
    let math: [MathSnippet]

    @Environment(\.colorScheme) private var colorScheme

    private static let indent: CGFloat = 22

    var body: some View {
        Group {
            switch block.kind {
            case .heading(let level):
                VStack(alignment: .leading, spacing: 8) {
                    text(block.text, size: Self.headingSize(level))
                        .font(.system(size: Self.headingSize(level), weight: level <= 2 ? .bold : .semibold))
                    if level <= 2 { Divider().opacity(0.6) }
                }
            case .paragraph:
                paragraph
            case .codeBlock(let language):
                CodeBlockView(code: block.code, language: language)
                    .padding(.leading, CGFloat(block.listDepth) * Self.indent)
            case .thematicBreak:
                Divider()
            case .table:
                TableBlockView(rows: block.tableRows) { cell, size in text(cell, size: size) }
            case .math(let index):
                MathBlockView(snippet: math.indices.contains(index) ? math[index] : nil)
                    .padding(.leading, CGFloat(block.listDepth) * Self.indent)
            }
        }
        .modifier(QuoteStyle(depth: block.quoteDepth))
    }

    /// 没有行内公式时直接用 Text，有的话才去拼图片
    private func text(_ attributed: AttributedString, size: CGFloat) -> Text {
        guard block.hasInlineMath else { return Text(attributed) }
        return InlineMath.text(attributed, math: math, fontSize: size, dark: colorScheme == .dark)
    }

    private var paragraph: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            if block.listDepth > 0 {
                Text(block.listMarker ?? "")
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
                    .frame(width: Self.indent - 6, alignment: .trailing)
            }
            text(block.text, size: 14)
                .lineSpacing(5)
                .fixedSize(horizontal: false, vertical: true)
        }
        .font(.system(size: 14))
        .padding(.leading, CGFloat(max(block.listDepth - 1, 0)) * Self.indent)
    }

    static func headingSize(_ level: Int) -> CGFloat {
        switch level {
        case 1: 26
        case 2: 21
        case 3: 17
        case 4: 15
        default: 14
        }
    }
}

private struct QuoteStyle: ViewModifier {
    let depth: Int

    func body(content: Content) -> some View {
        if depth == 0 {
            content
        } else {
            content
                .foregroundStyle(.secondary)
                .padding(.leading, 14)
                .overlay(alignment: .leading) {
                    RoundedRectangle(cornerRadius: 1.5)
                        .fill(.quaternary)
                        .frame(width: 3)
                }
        }
    }
}

private struct CodeBlockView: View {
    let code: String
    let language: String?

    @State private var copied = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text(language ?? "代码")
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(.secondary)
                Spacer()
                Button {
                    Pasteboard.writeString(code)
                    copied = true
                    Task {
                        try? await Task.sleep(for: .seconds(1.5))
                        copied = false
                    }
                } label: {
                    Label(copied ? "已复制" : "复制", systemImage: copied ? "checkmark" : "doc.on.doc")
                        .font(.system(size: 11))
                }
                .buttonStyle(.borderless)
            }
            .padding(.horizontal, 12)
            .padding(.top, 8)

            ScrollView(.horizontal, showsIndicators: false) {
                Text(code)
                    .font(.system(size: 12.5, design: .monospaced))
                    .lineSpacing(3)
                    .fixedSize()
                    .padding(.horizontal, 12)
                    .padding(.top, 6)
                    .padding(.bottom, 12)
            }
        }
        .background(Color.primary.opacity(0.045), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(Color.primary.opacity(0.08)))
    }
}

private struct TableBlockView: View {
    let rows: [[AttributedString]]
    /// 单元格的文字怎么画（里面可能有行内公式）
    let cellText: (AttributedString, CGFloat) -> Text

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            Grid(alignment: .leading, horizontalSpacing: 0, verticalSpacing: 0) {
                ForEach(rows.indices, id: \.self) { row in
                    GridRow {
                        ForEach(rows[row].indices, id: \.self) { column in
                            cellText(rows[row][column], 13)
                                .font(.system(size: 13, weight: row == 0 ? .semibold : .regular))
                                .frame(maxWidth: 320, alignment: .leading)
                                .padding(.horizontal, 12)
                                .padding(.vertical, 7)
                                .frame(maxHeight: .infinity, alignment: .topLeading)
                                .background(Self.background(for: row))
                        }
                    }
                    if row < rows.count - 1 {
                        Divider().gridCellUnsizedAxes(.horizontal)
                    }
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(Color.primary.opacity(0.1)))
        }
    }

    static func background(for row: Int) -> Color {
        if row == 0 { return Color.primary.opacity(0.06) }
        return row.isMultiple(of: 2) ? Color.primary.opacity(0.025) : .clear
    }
}

// MARK: - 编辑器

/// 纯文本编辑器。关掉智能引号、破折号替换和自动纠错，免得改坏 Markdown 符号。
struct MarkdownTextView: NSViewRepresentable {
    @Binding var text: String

    func makeCoordinator() -> Coordinator { Coordinator(text: $text) }

    func makeNSView(context: Context) -> NSScrollView {
        let scrollView = NSTextView.scrollableTextView()
        scrollView.drawsBackground = false
        guard let textView = scrollView.documentView as? NSTextView else { return scrollView }

        textView.delegate = context.coordinator
        textView.isRichText = false
        textView.importsGraphics = false
        textView.allowsUndo = true
        textView.isAutomaticQuoteSubstitutionEnabled = false
        textView.isAutomaticDashSubstitutionEnabled = false
        textView.isAutomaticTextReplacementEnabled = false
        textView.isAutomaticSpellingCorrectionEnabled = false
        textView.isContinuousSpellCheckingEnabled = false
        textView.smartInsertDeleteEnabled = false
        textView.drawsBackground = false
        textView.textContainerInset = NSSize(width: 24, height: 20)
        textView.typingAttributes = Self.attributes
        textView.string = text
        Self.applyAttributes(to: textView)
        return scrollView
    }

    func updateNSView(_ scrollView: NSScrollView, context: Context) {
        context.coordinator.text = $text
        guard let textView = scrollView.documentView as? NSTextView, textView.string != text else { return }
        let selection = textView.selectedRange()
        textView.string = text
        Self.applyAttributes(to: textView)
        let length = (text as NSString).length
        textView.setSelectedRange(NSRange(location: min(selection.location, length), length: 0))
    }

    private static let attributes: [NSAttributedString.Key: Any] = {
        let style = NSMutableParagraphStyle()
        style.lineHeightMultiple = 1.3
        return [
            .font: NSFont.monospacedSystemFont(ofSize: 13, weight: .regular),
            .foregroundColor: NSColor.labelColor,
            .paragraphStyle: style,
        ]
    }()

    private static func applyAttributes(to textView: NSTextView) {
        guard let storage = textView.textStorage else { return }
        storage.setAttributes(attributes, range: NSRange(location: 0, length: storage.length))
    }

    @MainActor
    final class Coordinator: NSObject, NSTextViewDelegate {
        var text: Binding<String>

        init(text: Binding<String>) { self.text = text }

        func textDidChange(_ notification: Notification) {
            guard let textView = notification.object as? NSTextView else { return }
            text.wrappedValue = textView.string
        }
    }
}
