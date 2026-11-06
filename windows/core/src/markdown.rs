//! 预览用的 Markdown 解析：把文档拆成块（标题、段落、列表项、代码块、分隔线、表格、公式），
//! 每块带行内格式（粗体、斜体、行内代码、删除线、链接），交给界面排版。
//!
//! macOS 版用 Foundation 的 Markdown 解析器，这里用 pulldown-cmark（同样是 CommonMark + GFM 表格、任务列表）。

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use crate::math_extractor::{self, BLOCK_CLOSE, BLOCK_OPEN, INLINE_OPEN, MathSnippet};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpanStyle {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    pub link: Option<String>,
}

/// 一段同样格式的文字
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: SpanStyle,
}

pub type Inline = Vec<Span>;

pub fn plain_text(inline: &Inline) -> String {
    inline.iter().map(|s| s.text.as_str()).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Paragraph,
    Heading(u8),
    CodeBlock(Option<String>),
    ThematicBreak,
    Table,
    /// 独立成段的公式，序号对应 `MarkdownDocument::math`
    Math(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkdownBlock {
    pub id: usize,
    pub kind: BlockKind,
    /// 段落、标题的内容；代码块是原文
    pub text: Inline,
    /// 列表嵌套层数，0 表示不在列表里
    pub list_depth: usize,
    /// 列表项第一段前面的符号（"•"、"1."）；同一项的后续段落为 None
    pub list_marker: Option<String>,
    pub quote_depth: usize,
    /// 表格的单元格，第一行是表头
    pub table_rows: Vec<Vec<Inline>>,
    pub table_align: Vec<Align>,
    /// 文字里有行内公式的占位符，排版时要换成公式
    pub has_inline_math: bool,
    /// 任务列表的勾选框：Some(true) 已完成
    pub task: Option<bool>,
}

impl MarkdownBlock {
    fn new(id: usize, kind: BlockKind) -> Self {
        MarkdownBlock {
            id,
            kind,
            text: Vec::new(),
            list_depth: 0,
            list_marker: None,
            quote_depth: 0,
            table_rows: Vec::new(),
            table_align: Vec::new(),
            has_inline_math: false,
            task: None,
        }
    }

    pub fn plain(&self) -> String {
        plain_text(&self.text)
    }

    /// 代码块的内容，去掉结尾的换行
    pub fn code(&self) -> String {
        let mut s = self.plain();
        if s.ends_with('\n') {
            s.pop();
        }
        s
    }
}

/// 解析结果：块，加上文档里所有的公式
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkdownDocument {
    pub blocks: Vec<MarkdownBlock>,
    pub math: Vec<MathSnippet>,
}

/// 先把公式换成占位符再解析，见 `math_extractor`
pub fn parse(source: &str) -> MarkdownDocument {
    let (markdown, math) = math_extractor::extract(source);
    let mut blocks = blocks_from(&markdown);
    if math.is_empty() {
        return MarkdownDocument { blocks, math };
    }
    for block in &mut blocks {
        if block.kind == BlockKind::Paragraph {
            if let Some(index) = block_math_index(&block.plain()).filter(|&i| i < math.len()) {
                block.kind = BlockKind::Math(index);
                continue;
            }
        }
        block.has_inline_math = if block.kind == BlockKind::Table {
            block.table_rows.iter().flatten().any(|cell| plain_text(cell).contains(INLINE_OPEN))
        } else {
            block.plain().contains(INLINE_OPEN)
        };
    }
    MarkdownDocument { blocks, math }
}

/// 整段只有一个 `\u{E002}序号\u{E003}`
fn block_math_index(plain: &str) -> Option<usize> {
    let trimmed = plain.trim();
    trimmed.strip_prefix(BLOCK_OPEN)?.strip_suffix(BLOCK_CLOSE)?.parse().ok()
}

struct ListState {
    ordered: bool,
    next: u64,
}

#[derive(Default)]
struct Builder {
    blocks: Vec<MarkdownBlock>,
    current: Option<usize>,
    styles: Vec<SpanStyle>,
    quote_depth: usize,
    lists: Vec<ListState>,
    /// 每层列表项还没用掉的符号
    pending_markers: Vec<Option<String>>,
    table: Option<usize>,
    in_code: bool,
}

impl Builder {
    fn open(&mut self, kind: BlockKind) -> usize {
        let id = self.blocks.len();
        let mut block = MarkdownBlock::new(id, kind);
        block.list_depth = self.lists.len();
        block.quote_depth = self.quote_depth;
        block.list_marker = self.pending_markers.last_mut().and_then(Option::take);
        self.blocks.push(block);
        self.current = Some(id);
        id
    }

    fn style(&self) -> SpanStyle {
        self.styles.last().cloned().unwrap_or_default()
    }

    /// 文字放到哪：表格里放进当前单元格，否则放进当前块（列表项里的文字可能没有段落包着）
    fn target(&mut self) -> &mut Inline {
        if let Some(table) = self.table {
            let rows = &mut self.blocks[table].table_rows;
            if rows.is_empty() {
                rows.push(Vec::new());
            }
            let row = rows.last_mut().unwrap();
            if row.is_empty() {
                row.push(Vec::new());
            }
            return row.last_mut().unwrap();
        }
        let index = match self.current {
            Some(i) => i,
            None => self.open(BlockKind::Paragraph),
        };
        &mut self.blocks[index].text
    }

    fn push_text(&mut self, text: &str, style: SpanStyle) {
        let target = self.target();
        if let Some(last) = target.last_mut() {
            if last.style == style {
                last.text.push_str(text);
                return;
            }
        }
        target.push(Span { text: text.to_string(), style });
    }

    fn push_style(&mut self, change: impl FnOnce(&mut SpanStyle)) {
        let mut style = self.style();
        change(&mut style);
        self.styles.push(style);
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => {
                if self.in_code {
                    if let Some(i) = self.current {
                        let block = &mut self.blocks[i];
                        match block.text.last_mut() {
                            Some(span) => span.text.push_str(&text),
                            None => block.text.push(Span { text: text.to_string(), style: SpanStyle::default() }),
                        }
                    }
                } else {
                    let style = self.style();
                    self.push_text(&text, style);
                }
            }
            Event::Code(text) => {
                let mut style = self.style();
                style.code = true;
                self.push_text(&text, style);
            }
            Event::InlineMath(text) | Event::DisplayMath(text) | Event::Html(text) | Event::InlineHtml(text) => {
                let style = self.style();
                self.push_text(&text, style);
            }
            Event::SoftBreak => {
                let style = self.style();
                self.push_text(" ", style);
            }
            Event::HardBreak => {
                let style = self.style();
                self.push_text("\n", style);
            }
            Event::Rule => {
                self.current = None;
                self.open(BlockKind::ThematicBreak);
                self.current = None;
            }
            Event::TaskListMarker(checked) => {
                let _ = self.target();
                if let Some(i) = self.current {
                    self.blocks[i].task = Some(checked);
                }
            }
            Event::FootnoteReference(name) => {
                let style = self.style();
                self.push_text(&format!("[^{name}]"), style);
            }
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph | Tag::HtmlBlock => {
                if self.table.is_none() {
                    self.open(BlockKind::Paragraph);
                }
            }
            Tag::Heading { level, .. } => {
                self.open(BlockKind::Heading(level as u8));
            }
            Tag::CodeBlock(kind) => {
                let language = match kind {
                    CodeBlockKind::Fenced(info) => info.split_whitespace().next().map(str::to_string).filter(|s| !s.is_empty()),
                    CodeBlockKind::Indented => None,
                };
                self.open(BlockKind::CodeBlock(language));
                self.in_code = true;
            }
            Tag::BlockQuote(_) => {
                self.current = None;
                self.quote_depth += 1;
            }
            Tag::List(start) => {
                self.current = None;
                self.lists.push(ListState { ordered: start.is_some(), next: start.unwrap_or(1) });
            }
            Tag::Item => {
                self.current = None;
                let depth = self.lists.len();
                let marker = match self.lists.last_mut() {
                    Some(list) if list.ordered => {
                        let m = format!("{}.", list.next);
                        list.next += 1;
                        m
                    }
                    _ => bullet(depth).to_string(),
                };
                self.pending_markers.push(Some(marker));
            }
            Tag::Table(alignments) => {
                self.current = None;
                let index = self.open(BlockKind::Table);
                self.blocks[index].table_align = alignments
                    .iter()
                    .map(|a| match a {
                        pulldown_cmark::Alignment::None => Align::None,
                        pulldown_cmark::Alignment::Left => Align::Left,
                        pulldown_cmark::Alignment::Center => Align::Center,
                        pulldown_cmark::Alignment::Right => Align::Right,
                    })
                    .collect();
                self.table = Some(index);
                self.current = None;
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(table) = self.table {
                    self.blocks[table].table_rows.push(Vec::new());
                }
            }
            Tag::TableCell => {
                if let Some(table) = self.table {
                    if let Some(row) = self.blocks[table].table_rows.last_mut() {
                        row.push(Vec::new());
                    }
                }
            }
            Tag::Emphasis => self.push_style(|s| s.italic = true),
            Tag::Strong => self.push_style(|s| s.bold = true),
            Tag::Strikethrough => self.push_style(|s| s.strike = true),
            Tag::Link { dest_url, .. } => {
                let url = dest_url.to_string();
                self.push_style(|s| s.link = Some(url));
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::HtmlBlock => {
                if self.table.is_none() {
                    self.current = None;
                }
            }
            TagEnd::CodeBlock => {
                self.in_code = false;
                self.current = None;
            }
            TagEnd::BlockQuote(_) => {
                self.quote_depth = self.quote_depth.saturating_sub(1);
                self.current = None;
            }
            TagEnd::List(_) => {
                self.lists.pop();
                self.current = None;
            }
            TagEnd::Item => {
                self.pending_markers.pop();
                self.current = None;
            }
            TagEnd::Table => {
                self.table = None;
                self.current = None;
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                self.styles.pop();
            }
            _ => {}
        }
    }
}

/// 只解析 Markdown，不处理公式
pub fn blocks_from(source: &str) -> Vec<MarkdownBlock> {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut builder = Builder::default();
    for event in Parser::new_ext(source, options) {
        builder.event(event);
    }
    builder.blocks
}

pub fn bullet(depth: usize) -> &'static str {
    match depth {
        0 | 1 => "•",
        2 => "◦",
        _ => "▪",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_and_paragraphs() {
        let blocks = blocks_from("# 标题\n\n第一段 **粗体**\n第二行");
        assert_eq!(blocks.iter().map(|b| b.kind.clone()).collect::<Vec<_>>(), [BlockKind::Heading(1), BlockKind::Paragraph]);
        assert_eq!(blocks[1].plain(), "第一段 粗体 第二行");
        assert!(blocks[1].text.iter().any(|s| s.style.bold && s.text == "粗体"));
    }

    #[test]
    fn nested_lists_get_markers_and_depth() {
        let blocks = blocks_from("- 一\n  - 嵌套\n- 二\n\n1. 甲\n2. 乙");
        assert_eq!(blocks.iter().map(|b| b.list_depth).collect::<Vec<_>>(), [1, 2, 1, 1, 1]);
        assert_eq!(
            blocks.iter().map(|b| b.list_marker.as_deref()).collect::<Vec<_>>(),
            [Some("•"), Some("◦"), Some("•"), Some("1."), Some("2.")]
        );
    }

    #[test]
    fn second_paragraph_in_list_item_has_no_marker() {
        let blocks = blocks_from("- 第一段\n\n  同一项的第二段\n- 下一项");
        assert_eq!(blocks.iter().map(|b| b.list_marker.as_deref()).collect::<Vec<_>>(), [Some("•"), None, Some("•")]);
    }

    #[test]
    fn code_block_keeps_language_and_drops_trailing_newline() {
        let blocks = blocks_from("```swift\nlet a = 1\n```");
        assert_eq!(blocks[0].kind, BlockKind::CodeBlock(Some("swift".into())));
        assert_eq!(blocks[0].code(), "let a = 1");
    }

    #[test]
    fn quote_and_rule() {
        let blocks = blocks_from("> 引用\n\n---\n\n正文");
        assert_eq!(
            blocks.iter().map(|b| b.kind.clone()).collect::<Vec<_>>(),
            [BlockKind::Paragraph, BlockKind::ThematicBreak, BlockKind::Paragraph]
        );
        assert_eq!(blocks[0].quote_depth, 1);
        assert_eq!(blocks[2].quote_depth, 0);
    }

    #[test]
    fn table_cells_are_collected() {
        let blocks = blocks_from("| A | B |\n|---|:-:|\n| 1 | 2 |\n| 3 | 4 |");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, BlockKind::Table);
        let rows: Vec<Vec<String>> = blocks[0].table_rows.iter().map(|r| r.iter().map(plain_text).collect()).collect();
        assert_eq!(rows, [["A", "B"], ["1", "2"], ["3", "4"]]);
        assert_eq!(blocks[0].table_align, [Align::None, Align::Center]);
    }

    #[test]
    fn ids_are_unique_and_ordered() {
        let blocks = blocks_from("# A\n\n| x |\n|---|\n| y |\n\n段落");
        assert_eq!(blocks.iter().map(|b| b.id).collect::<Vec<_>>(), (0..blocks.len()).collect::<Vec<_>>());
    }

    #[test]
    fn task_list_and_inline_styles() {
        let blocks = blocks_from("- [x] 完成 `code` [链接](https://a.b)\n- [ ] 待办 ~~删掉~~");
        assert_eq!(blocks[0].task, Some(true));
        assert_eq!(blocks[1].task, Some(false));
        assert!(blocks[0].text.iter().any(|s| s.style.code && s.text == "code"));
        assert!(blocks[0].text.iter().any(|s| s.style.link.as_deref() == Some("https://a.b")));
        assert!(blocks[1].text.iter().any(|s| s.style.strike));
    }
}
