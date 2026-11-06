//! LaTeX 公式排版：用 Windows 自带的 Cambria Math 字体，原生绘制，不嵌网页、不用下载字体。
//!
//! 支持常用的写法：分数、根号、上下标、求和/积分/极限的上下限、\left( \right) 自动拉伸的括号、
//! 矩阵（pmatrix、bmatrix、vmatrix……）、cases、aligned、希腊字母、\mathbb/\mathcal/\mathbf、
//! 各种关系符号和箭头、重音（\hat、\bar、\vec……）、\text{中文}。不认识的命令按原文显示。
//!
//! 排版规则参照 TeX 做了简化：字母用数学斜体（Unicode 数学字母区，Cambria Math 都有），
//! 二元运算符、关系符号前后加不同的间距，上下标缩小到 70%。

use std::cell::RefCell;
use std::collections::HashMap;

use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::core::{BOOL, HSTRING};
use windows_numerics::Matrix3x2;

use crate::gfx::{Color, Painter, Rect, v2};
use crate::text::{self, Font, TextOpts};

// MARK: 字体

#[derive(Clone, Copy)]
struct Glyph {
    index: u16,
    advance: f32,
    top: f32,
    bottom: f32,
}

struct MathFont {
    face: IDWriteFontFace,
    units: f32,
    glyphs: HashMap<u32, Option<Glyph>>,
}

thread_local! {
    static FONT: RefCell<Option<Option<MathFont>>> = const { RefCell::new(None) };
}

fn with_font<R>(f: impl FnOnce(&mut MathFont) -> R) -> Option<R> {
    FONT.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Some(load_font());
        }
        slot.as_mut().unwrap().as_mut().map(f)
    })
}

fn load_font() -> Option<MathFont> {
    unsafe {
        let factory = text::dwrite();
        let mut collection = None;
        factory.GetSystemFontCollection(&mut collection, false).ok()?;
        let collection = collection?;
        let mut index = 0u32;
        let mut exists = BOOL(0);
        collection.FindFamilyName(&HSTRING::from("Cambria Math"), &mut index, &mut exists).ok()?;
        if !exists.as_bool() {
            return None;
        }
        let family = collection.GetFontFamily(index).ok()?;
        let font = family.GetFirstMatchingFont(DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL).ok()?;
        let face = font.CreateFontFace().ok()?;
        let mut metrics = DWRITE_FONT_METRICS::default();
        face.GetMetrics(&mut metrics);
        Some(MathFont { face, units: metrics.designUnitsPerEm as f32, glyphs: HashMap::new() })
    }
}

/// 字形的度量（以 em 为单位）；字体里没有这个字返回 None
fn glyph(cp: u32) -> Option<Glyph> {
    with_font(|font| {
        *font.glyphs.entry(cp).or_insert_with(|| unsafe {
            let mut index = 0u16;
            font.face.GetGlyphIndices(&cp, 1, &mut index).ok()?;
            if index == 0 {
                return None;
            }
            let mut m = DWRITE_GLYPH_METRICS::default();
            font.face.GetDesignGlyphMetrics(&index, 1, &mut m, false).ok()?;
            let y_max = (m.verticalOriginY - m.topSideBearing) as f32;
            let y_min = (m.verticalOriginY - m.advanceHeight as i32 + m.bottomSideBearing) as f32;
            Some(Glyph { index, advance: m.advanceWidth as f32 / font.units, top: y_max / font.units, bottom: -y_min / font.units })
        })
    })
    .flatten()
}

pub fn available() -> bool {
    with_font(|_| ()).is_some()
}

// MARK: 排好的结果

#[derive(Clone, Debug)]
enum Cmd {
    /// 一个字形，(x, y) 是基线起点
    Glyph { x: f32, y: f32, size: f32, index: u16, scale_y: f32 },
    /// 字体里没有的字（中文等）用界面字体排
    Text { x: f32, y: f32, size: f32, text: String },
    Rule { x: f32, y: f32, w: f32, h: f32 },
    Path { points: Vec<(f32, f32)>, width: f32 },
    Dot { x: f32, y: f32, r: f32 },
}

/// 排好版的公式：原点在基线左端，y 向下为正
#[derive(Clone, Debug, Default)]
pub struct MathLayout {
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
    cmds: Vec<Cmd>,
}

impl MathLayout {
    fn empty() -> MathLayout {
        MathLayout::default()
    }

    fn shifted(mut self, dx: f32, dy: f32) -> MathLayout {
        for cmd in &mut self.cmds {
            match cmd {
                Cmd::Glyph { x, y, .. } | Cmd::Text { x, y, .. } | Cmd::Rule { x, y, .. } | Cmd::Dot { x, y, .. } => {
                    *x += dx;
                    *y += dy;
                }
                Cmd::Path { points, .. } => {
                    for p in points {
                        p.0 += dx;
                        p.1 += dy;
                    }
                }
            }
        }
        self
    }

    /// 把 `other` 放在 (dx, dy)，合并
    fn place(&mut self, other: MathLayout, dx: f32, dy: f32) {
        self.ascent = self.ascent.max(other.ascent - dy);
        self.descent = self.descent.max(other.descent + dy);
        self.width = self.width.max(dx + other.width);
        self.cmds.extend(other.shifted(dx, dy).cmds);
    }

    pub fn height(&self) -> f32 {
        self.ascent + self.descent
    }

    /// 画在 (x, baseline)
    pub fn draw(&self, p: &Painter, x: f32, baseline: f32, color: Color) {
        let face = FONT.with(|cell| cell.borrow().as_ref().and_then(|f| f.as_ref().map(|f| f.face.clone())));
        for cmd in &self.cmds {
            match cmd {
                Cmd::Glyph { x: gx, y, size, index, scale_y } => {
                    let Some(face) = &face else { continue };
                    let run = DWRITE_GLYPH_RUN {
                        fontFace: std::mem::ManuallyDrop::new(Some(face.clone())),
                        fontEmSize: *size,
                        glyphCount: 1,
                        glyphIndices: index,
                        glyphAdvances: std::ptr::null(),
                        glyphOffsets: std::ptr::null(),
                        isSideways: false.into(),
                        bidiLevel: 0,
                    };
                    let (ox, oy) = (x + gx, baseline + y);
                    unsafe {
                        if (*scale_y - 1.0).abs() > 0.01 {
                            let m = Matrix3x2::scale_around(1.0, *scale_y, windows_numerics::Vector2 { X: ox, Y: oy });
                            p.transform(m, || p.ctx.DrawGlyphRun(v2(ox, oy), &run, None, p.brush(color), DWRITE_MEASURING_MODE_NATURAL));
                        } else {
                            p.ctx.DrawGlyphRun(v2(ox, oy), &run, None, p.brush(color), DWRITE_MEASURING_MODE_NATURAL);
                        }
                        std::mem::ManuallyDrop::into_inner(run.fontFace);
                    }
                }
                Cmd::Text { x: tx, y, size, text: s } => {
                    if let Some(l) = text::layout(s, Font::ui(*size, 400), 10_000.0, size * 3.0, TextOpts { ellipsis: false, ..TextOpts::WRAP }) {
                        p.draw_layout(&l.layout, x + tx, baseline + y - l.baseline, color);
                    }
                }
                Cmd::Rule { x: rx, y, w, h } => p.fill_rect(Rect::new(x + rx, baseline + y, *w, h.max(0.8)), color),
                Cmd::Path { points, width } => {
                    let pts: Vec<(f32, f32)> = points.iter().map(|(px, py)| (x + px, baseline + py)).collect();
                    p.polyline(&pts, color, *width, false);
                }
                Cmd::Dot { x: dx, y, r } => p.fill_circle(x + dx, baseline + y, *r, color),
            }
        }
    }
}

// MARK: 语法树

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Ord,
    Op,
    Bin,
    Rel,
    Open,
    Close,
    Punct,
    Inner,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Variant {
    Normal,
    Italic,
    Bold,
    Double,
    Script,
    Sans,
    Mono,
}

#[derive(Clone, Debug)]
enum Node {
    Sym(char, Class),
    /// 一串字母数字，按字体样式映射
    Chars(String, Variant),
    Group(Vec<Node>),
    Frac { num: Box<Node>, den: Box<Node>, bar: bool, size: Option<bool> },
    Sqrt { index: Option<Box<Node>>, body: Box<Node> },
    Scripts { base: Box<Node>, sup: Option<Box<Node>>, sub: Option<Box<Node>> },
    LeftRight { left: Option<char>, body: Vec<Node>, right: Option<char> },
    Big { delim: char, scale: f32, class: Class },
    Text(String),
    Func(String, bool),
    BigOp(char, bool),
    Accent { kind: Accent, body: Box<Node> },
    Space(f32),
    Matrix { rows: Vec<Vec<Node>>, kind: MatrixKind },
    Over { over: Box<Node>, body: Box<Node> },
    Under { under: Box<Node>, body: Box<Node> },
    Boxed(Box<Node>),
    Raw(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Accent {
    Hat,
    Tilde,
    Bar,
    Vec,
    Dot,
    Ddot,
    Underline,
    Check,
    Breve,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MatrixKind {
    Plain,
    Paren,
    Bracket,
    Brace,
    Vert,
    DoubleVert,
    Cases,
    Aligned,
}

// MARK: 解析

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Cmd(String),
    Char(char),
    Open,
    Close,
    Sup,
    Sub,
    Amp,
    Newline,
}

fn tokenize(src: &str) -> Vec<Token> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                i += 1;
                if i >= chars.len() {
                    break;
                }
                if chars[i].is_ascii_alphabetic() {
                    let start = i;
                    while i < chars.len() && chars[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    out.push(Token::Cmd(chars[start..i].iter().collect()));
                    continue;
                }
                if chars[i] == '\\' {
                    out.push(Token::Newline);
                } else {
                    out.push(Token::Cmd(chars[i].to_string()));
                }
            }
            '{' => out.push(Token::Open),
            '}' => out.push(Token::Close),
            '^' => out.push(Token::Sup),
            '_' => out.push(Token::Sub),
            '&' => out.push(Token::Amp),
            '%' => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            c if c.is_whitespace() => {}
            '\'' => out.push(Token::Char('′')),
            c => out.push(Token::Char(c)),
        }
        i += 1;
    }
    out
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

fn symbol(name: &str) -> Option<(char, Class)> {
    use Class::*;
    Some(match name {
        "alpha" => ('α', Ord), "beta" => ('β', Ord), "gamma" => ('γ', Ord), "delta" => ('δ', Ord),
        "epsilon" => ('ϵ', Ord), "varepsilon" => ('ε', Ord), "zeta" => ('ζ', Ord), "eta" => ('η', Ord),
        "theta" => ('θ', Ord), "vartheta" => ('ϑ', Ord), "iota" => ('ι', Ord), "kappa" => ('κ', Ord),
        "lambda" => ('λ', Ord), "mu" => ('μ', Ord), "nu" => ('ν', Ord), "xi" => ('ξ', Ord), "omicron" => ('ο', Ord),
        "pi" => ('π', Ord), "varpi" => ('ϖ', Ord), "rho" => ('ρ', Ord), "varrho" => ('ϱ', Ord), "sigma" => ('σ', Ord),
        "varsigma" => ('ς', Ord), "tau" => ('τ', Ord), "upsilon" => ('υ', Ord), "phi" => ('ϕ', Ord), "varphi" => ('φ', Ord),
        "chi" => ('χ', Ord), "psi" => ('ψ', Ord), "omega" => ('ω', Ord),
        "Gamma" => ('Γ', Ord), "Delta" => ('Δ', Ord), "Theta" => ('Θ', Ord), "Lambda" => ('Λ', Ord), "Xi" => ('Ξ', Ord),
        "Pi" => ('Π', Ord), "Sigma" => ('Σ', Ord), "Upsilon" => ('Υ', Ord), "Phi" => ('Φ', Ord), "Psi" => ('Ψ', Ord), "Omega" => ('Ω', Ord),
        "infty" => ('∞', Ord), "partial" => ('∂', Ord), "nabla" => ('∇', Ord), "hbar" => ('ℏ', Ord), "ell" => ('ℓ', Ord),
        "Re" => ('ℜ', Ord), "Im" => ('ℑ', Ord), "aleph" => ('ℵ', Ord), "emptyset" => ('∅', Ord), "varnothing" => ('∅', Ord),
        "forall" => ('∀', Ord), "exists" => ('∃', Ord), "nexists" => ('∄', Ord), "neg" => ('¬', Ord), "lnot" => ('¬', Ord),
        "angle" => ('∠', Ord), "triangle" => ('△', Ord), "prime" => ('′', Ord), "degree" => ('°', Ord), "top" => ('⊤', Ord),
        "bot" => ('⊥', Ord), "ldots" => ('…', Inner), "dots" => ('…', Inner), "cdots" => ('⋯', Inner), "vdots" => ('⋮', Ord),
        "ddots" => ('⋱', Inner), "dotsc" => ('…', Inner), "dotsb" => ('⋯', Inner), "checkmark" => ('✓', Ord),
        "pm" => ('±', Bin), "mp" => ('∓', Bin), "times" => ('×', Bin), "div" => ('÷', Bin), "cdot" => ('⋅', Bin),
        "ast" => ('∗', Bin), "star" => ('⋆', Bin), "circ" => ('∘', Bin), "bullet" => ('∙', Bin), "cup" => ('∪', Bin),
        "cap" => ('∩', Bin), "setminus" => ('∖', Bin), "land" => ('∧', Bin), "lor" => ('∨', Bin), "wedge" => ('∧', Bin),
        "vee" => ('∨', Bin), "oplus" => ('⊕', Bin), "ominus" => ('⊖', Bin), "otimes" => ('⊗', Bin), "odot" => ('⊙', Bin),
        "le" | "leq" => ('≤', Rel), "ge" | "geq" => ('≥', Rel), "ne" | "neq" => ('≠', Rel), "approx" => ('≈', Rel),
        "equiv" => ('≡', Rel), "sim" => ('∼', Rel), "simeq" => ('≃', Rel), "cong" => ('≅', Rel), "propto" => ('∝', Rel),
        "ll" => ('≪', Rel), "gg" => ('≫', Rel), "in" => ('∈', Rel), "notin" => ('∉', Rel), "ni" => ('∋', Rel),
        "subset" => ('⊂', Rel), "supset" => ('⊃', Rel), "subseteq" => ('⊆', Rel), "supseteq" => ('⊇', Rel),
        "to" | "rightarrow" => ('→', Rel), "leftarrow" | "gets" => ('←', Rel), "Rightarrow" => ('⇒', Rel),
        "Leftarrow" => ('⇐', Rel), "leftrightarrow" => ('↔', Rel), "Leftrightarrow" => ('⇔', Rel), "mapsto" => ('↦', Rel),
        "implies" => ('⟹', Rel), "iff" => ('⟺', Rel), "longrightarrow" => ('⟶', Rel), "longleftarrow" => ('⟵', Rel),
        "uparrow" => ('↑', Rel), "downarrow" => ('↓', Rel), "perp" => ('⊥', Rel), "parallel" => ('∥', Rel), "mid" => ('∣', Rel),
        "vdash" => ('⊢', Rel), "models" => ('⊨', Rel), "prec" => ('≺', Rel), "succ" => ('≻', Rel), "doteq" => ('≐', Rel),
        "leqslant" => ('⩽', Rel), "geqslant" => ('⩾', Rel), "lesssim" => ('≲', Rel), "gtrsim" => ('≳', Rel),
        "langle" => ('⟨', Open), "rangle" => ('⟩', Close), "lfloor" => ('⌊', Open), "rfloor" => ('⌋', Close),
        "lceil" => ('⌈', Open), "rceil" => ('⌉', Close), "lvert" => ('|', Open), "rvert" => ('|', Close),
        "lVert" => ('‖', Open), "rVert" => ('‖', Close), "vert" => ('|', Ord), "Vert" => ('‖', Ord),
        "{" | "lbrace" => ('{', Open), "}" | "rbrace" => ('}', Close), "|" => ('‖', Ord), "colon" => (':', Punct),
        "$" => ('$', Ord), "%" => ('%', Ord), "#" => ('#', Ord), "&" => ('&', Ord), "_" => ('_', Ord),
        _ => return None,
    })
}

fn big_op(name: &str) -> Option<(char, bool)> {
    Some(match name {
        "sum" => ('∑', true),
        "prod" => ('∏', true),
        "coprod" => ('∐', true),
        "int" => ('∫', false),
        "iint" => ('∬', false),
        "iiint" => ('∭', false),
        "oint" => ('∮', false),
        "bigcup" => ('⋃', true),
        "bigcap" => ('⋂', true),
        "bigoplus" => ('⨁', true),
        "bigotimes" => ('⨂', true),
        "bigvee" => ('⋁', true),
        "bigwedge" => ('⋀', true),
        _ => return None,
    })
}

fn function(name: &str) -> Option<bool> {
    Some(match name {
        "lim" | "max" | "min" | "sup" | "inf" | "det" | "Pr" | "gcd" | "limsup" | "liminf" | "argmax" | "argmin" => true,
        "sin" | "cos" | "tan" | "cot" | "sec" | "csc" | "arcsin" | "arccos" | "arctan" | "sinh" | "cosh" | "tanh" | "coth" | "log"
        | "ln" | "lg" | "exp" | "dim" | "ker" | "deg" | "arg" | "hom" | "mod" | "bmod" => false,
        _ => return None,
    })
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    /// 解析到 `}`、`&`、`\\`、`\right`、`\end` 或结尾
    fn expr(&mut self) -> Vec<Node> {
        let mut nodes: Vec<Node> = Vec::new();
        while let Some(token) = self.peek().cloned() {
            match token {
                Token::Close | Token::Amp | Token::Newline => break,
                Token::Cmd(ref name) if name == "right" || name == "end" => break,
                Token::Sup | Token::Sub => {
                    self.pos += 1;
                    let arg = self.argument();
                    let base = nodes.pop().unwrap_or(Node::Group(vec![]));
                    let (b, mut sup, mut sub) = match base {
                        Node::Scripts { base, sup, sub } => (*base, sup, sub),
                        other => (other, None, None),
                    };
                    if token == Token::Sup {
                        // x'^2 这种：把撇号并进上标
                        sup = Some(Box::new(match sup {
                            Some(existing) => Node::Group(vec![*existing, arg]),
                            None => arg,
                        }));
                    } else {
                        sub = Some(Box::new(arg));
                    }
                    nodes.push(Node::Scripts { base: Box::new(b), sup, sub });
                }
                Token::Char('′') => {
                    self.pos += 1;
                    let base = nodes.pop().unwrap_or(Node::Group(vec![]));
                    nodes.push(Node::Scripts { base: Box::new(base), sup: Some(Box::new(Node::Sym('′', Class::Ord))), sub: None });
                }
                _ => {
                    if let Some(node) = self.atom() {
                        nodes.push(node);
                    }
                }
            }
        }
        nodes
    }

    /// 上下标、命令的参数：一个 `{…}` 组或者一个记号
    fn argument(&mut self) -> Node {
        match self.peek() {
            Some(Token::Open) => {
                self.pos += 1;
                let body = self.expr();
                if self.peek() == Some(&Token::Close) {
                    self.pos += 1;
                }
                Node::Group(body)
            }
            Some(Token::Char(c)) => {
                let c = *c;
                self.pos += 1;
                char_node(c)
            }
            Some(_) => self.atom().unwrap_or(Node::Group(vec![])),
            None => Node::Group(vec![]),
        }
    }

    /// `{…}` 里的原文（\text、\begin 的环境名用）
    fn raw_group(&mut self) -> String {
        let mut out = String::new();
        if self.peek() != Some(&Token::Open) {
            if let Some(Token::Char(c)) = self.peek().cloned() {
                self.pos += 1;
                return c.to_string();
            }
            return out;
        }
        self.pos += 1;
        let mut depth = 1;
        while let Some(t) = self.next() {
            match t {
                Token::Open => {
                    depth += 1;
                    out.push('{');
                }
                Token::Close => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push('}');
                }
                Token::Char(c) => out.push(c),
                Token::Cmd(name) => {
                    match name.as_str() {
                        " " | "," | ";" | ":" | "quad" => out.push(' '),
                        _ => {
                            if let Some((c, _)) = symbol(&name) {
                                out.push(c);
                            } else {
                                out.push('\\');
                                out.push_str(&name);
                            }
                        }
                    }
                }
                Token::Sup => out.push('^'),
                Token::Sub => out.push('_'),
                Token::Amp => out.push('&'),
                Token::Newline => out.push(' '),
            }
        }
        out
    }

    /// `[…]` 可选参数
    fn optional(&mut self) -> Option<Node> {
        if self.peek() != Some(&Token::Char('[')) {
            return None;
        }
        self.pos += 1;
        let mut nodes = Vec::new();
        while let Some(t) = self.peek() {
            if *t == Token::Char(']') {
                self.pos += 1;
                break;
            }
            if matches!(t, Token::Close) {
                break;
            }
            match self.atom() {
                Some(n) => nodes.push(n),
                None => break,
            }
        }
        Some(Node::Group(nodes))
    }

    fn delimiter(&mut self) -> Option<char> {
        match self.next()? {
            Token::Char('.') => None,
            Token::Char(c) => Some(c),
            Token::Cmd(name) => symbol(&name).map(|(c, _)| c).or(match name.as_str() {
                "{" => Some('{'),
                "}" => Some('}'),
                "|" => Some('‖'),
                _ => None,
            }),
            _ => None,
        }
    }

    fn atom(&mut self) -> Option<Node> {
        let token = self.next()?;
        match token {
            Token::Open => {
                let body = self.expr();
                if self.peek() == Some(&Token::Close) {
                    self.pos += 1;
                }
                Some(Node::Group(body))
            }
            Token::Char(c) => Some(char_node(c)),
            Token::Cmd(name) => Some(self.command(&name)),
            Token::Close | Token::Amp | Token::Newline | Token::Sup | Token::Sub => None,
        }
    }

    fn command(&mut self, name: &str) -> Node {
        if let Some((c, class)) = symbol(name) {
            return Node::Sym(c, class);
        }
        if let Some((c, limits)) = big_op(name) {
            return Node::BigOp(c, limits);
        }
        if let Some(limits) = function(name) {
            return Node::Func(name.to_string(), limits);
        }
        match name {
            "frac" | "dfrac" | "tfrac" | "cfrac" => {
                let num = self.argument();
                let den = self.argument();
                let size = match name {
                    "dfrac" | "cfrac" => Some(true),
                    "tfrac" => Some(false),
                    _ => None,
                };
                Node::Frac { num: Box::new(num), den: Box::new(den), bar: true, size }
            }
            "binom" | "dbinom" | "tbinom" => {
                let num = self.argument();
                let den = self.argument();
                Node::LeftRight { left: Some('('), body: vec![Node::Frac { num: Box::new(num), den: Box::new(den), bar: false, size: None }], right: Some(')') }
            }
            "sqrt" => {
                let index = self.optional().map(Box::new);
                let body = self.argument();
                Node::Sqrt { index, body: Box::new(body) }
            }
            "left" => {
                let left = self.delimiter();
                let body = self.expr();
                let right = if self.peek() == Some(&Token::Cmd("right".into())) {
                    self.pos += 1;
                    self.delimiter()
                } else {
                    None
                };
                Node::LeftRight { left, body, right }
            }
            "big" | "Big" | "bigg" | "Bigg" | "bigl" | "bigr" | "Bigl" | "Bigr" | "biggl" | "biggr" | "Biggl" | "Biggr" | "bigm" | "Bigm" => {
                let scale = match name.trim_end_matches(['l', 'r', 'm']) {
                    "big" => 1.2,
                    "Big" => 1.6,
                    "bigg" => 2.0,
                    _ => 2.4,
                };
                let class = if name.ends_with('l') { Class::Open } else if name.ends_with('r') { Class::Close } else { Class::Ord };
                match self.delimiter() {
                    Some(delim) => Node::Big { delim, scale, class },
                    None => Node::Group(vec![]),
                }
            }
            "text" | "textrm" | "textnormal" | "mbox" | "textit" | "textbf" | "textsf" | "texttt" => Node::Text(self.raw_group()),
            "mathrm" | "operatorname" | "rm" | "mathup" => {
                let raw = self.raw_group();
                if name == "operatorname" { Node::Func(raw, false) } else { Node::Chars(raw, Variant::Normal) }
            }
            "mathbf" | "boldsymbol" | "bm" | "bf" => self.styled(Variant::Bold),
            "mathit" | "it" => self.styled(Variant::Italic),
            "mathbb" => self.styled(Variant::Double),
            "mathcal" | "mathscr" => self.styled(Variant::Script),
            "mathsf" => self.styled(Variant::Sans),
            "mathtt" => self.styled(Variant::Mono),
            "hat" | "widehat" => self.accent(Accent::Hat),
            "tilde" | "widetilde" => self.accent(Accent::Tilde),
            "bar" | "overline" => self.accent(Accent::Bar),
            "vec" | "overrightarrow" => self.accent(Accent::Vec),
            "dot" => self.accent(Accent::Dot),
            "ddot" => self.accent(Accent::Ddot),
            "underline" => self.accent(Accent::Underline),
            "check" => self.accent(Accent::Check),
            "breve" => self.accent(Accent::Breve),
            "overbrace" | "underbrace" => self.argument(),
            "overset" | "stackrel" => {
                let over = self.argument();
                let body = self.argument();
                Node::Over { over: Box::new(over), body: Box::new(body) }
            }
            "underset" => {
                let under = self.argument();
                let body = self.argument();
                Node::Under { under: Box::new(under), body: Box::new(body) }
            }
            "boxed" | "fbox" => Node::Boxed(Box::new(self.argument())),
            "phantom" | "vphantom" | "hphantom" => {
                self.argument();
                Node::Space(0.0)
            }
            "color" | "textcolor" => {
                self.raw_group();
                self.argument()
            }
            "mathrel" | "mathbin" | "mathop" | "mathord" | "mathopen" | "mathclose" | "displaystyle" | "textstyle" | "scriptstyle" | "limits" | "nolimits" | "left." => {
                Node::Group(vec![])
            }
            "not" => {
                let next = self.argument();
                Node::Group(vec![next, Node::Space(-0.55), Node::Sym('/', Class::Ord)])
            }
            "quad" => Node::Space(1.0),
            "qquad" => Node::Space(2.0),
            "," | "thinspace" => Node::Space(3.0 / 18.0),
            ":" | ">" | "medspace" => Node::Space(4.0 / 18.0),
            ";" | "thickspace" => Node::Space(5.0 / 18.0),
            "!" | "negthinspace" => Node::Space(-3.0 / 18.0),
            " " | "space" => Node::Space(0.33),
            "begin" => {
                let env = self.raw_group();
                self.environment(&env)
            }
            other => Node::Raw(format!("\\{other}")),
        }
    }

    fn styled(&mut self, variant: Variant) -> Node {
        let arg = self.argument();
        restyle(arg, variant)
    }

    fn accent(&mut self, kind: Accent) -> Node {
        let body = self.argument();
        Node::Accent { kind, body: Box::new(body) }
    }

    fn environment(&mut self, env: &str) -> Node {
        let kind = match env.trim_end_matches('*') {
            "pmatrix" => MatrixKind::Paren,
            "bmatrix" => MatrixKind::Bracket,
            "Bmatrix" => MatrixKind::Brace,
            "vmatrix" => MatrixKind::Vert,
            "Vmatrix" => MatrixKind::DoubleVert,
            "cases" | "dcases" => MatrixKind::Cases,
            "aligned" | "align" | "split" | "gathered" | "eqnarray" | "alignat" => MatrixKind::Aligned,
            "array" => {
                self.raw_group();
                MatrixKind::Plain
            }
            _ => MatrixKind::Plain,
        };
        let mut rows: Vec<Vec<Node>> = Vec::new();
        let mut row: Vec<Node> = Vec::new();
        loop {
            let cell = self.expr();
            row.push(Node::Group(cell));
            match self.next() {
                Some(Token::Amp) => continue,
                Some(Token::Newline) => {
                    rows.push(std::mem::take(&mut row));
                }
                Some(Token::Cmd(name)) if name == "end" => {
                    self.raw_group();
                    break;
                }
                Some(Token::Close) => continue,
                None => break,
                _ => {}
            }
        }
        if row.iter().any(|c| !matches!(c, Node::Group(v) if v.is_empty())) || rows.is_empty() {
            rows.push(row);
        }
        Node::Matrix { rows, kind }
    }
}

fn char_node(c: char) -> Node {
    use Class::*;
    match c {
        'a'..='z' | 'A'..='Z' => Node::Chars(c.to_string(), Variant::Italic),
        '0'..='9' | '.' => Node::Chars(c.to_string(), Variant::Normal),
        '+' | '*' => Node::Sym(if c == '*' { '∗' } else { '+' }, Bin),
        '-' => Node::Sym('−', Bin),
        '=' | '<' | '>' | ':' => Node::Sym(c, Rel),
        '(' | '[' => Node::Sym(c, Open),
        ')' | ']' => Node::Sym(c, Close),
        ',' | ';' => Node::Sym(c, Punct),
        '!' | '?' => Node::Sym(c, Close),
        '|' | '/' => Node::Sym(c, Ord),
        c if ('\u{3B1}'..='\u{3C9}').contains(&c) => Node::Sym(c, Ord),
        c => Node::Sym(c, Ord),
    }
}

fn restyle(node: Node, variant: Variant) -> Node {
    match node {
        Node::Chars(s, _) => Node::Chars(s, variant),
        Node::Group(nodes) => Node::Group(nodes.into_iter().map(|n| restyle(n, variant)).collect()),
        Node::Sym(c, class) if variant == Variant::Bold && c.is_alphabetic() => Node::Chars(c.to_string(), Variant::Bold).with_class(class),
        other => other,
    }
}

impl Node {
    fn with_class(self, _class: Class) -> Node {
        self
    }

    fn class(&self) -> Class {
        match self {
            Node::Sym(_, c) => *c,
            Node::BigOp(..) | Node::Func(..) => Class::Op,
            Node::Scripts { base, .. } => base.class(),
            Node::Big { class, .. } => *class,
            Node::LeftRight { .. } | Node::Frac { .. } => Class::Inner,
            Node::Space(_) => Class::Punct,
            _ => Class::Ord,
        }
    }
}

/// 字母映射到 Unicode 数学字母区
fn map_char(c: char, variant: Variant) -> char {
    let off = |base: u32, start: char| char::from_u32(base + (c as u32 - start as u32)).unwrap_or(c);
    match variant {
        Variant::Normal => c,
        Variant::Italic => match c {
            'h' => 'ℎ',
            'a'..='z' => off(0x1D44E, 'a'),
            'A'..='Z' => off(0x1D434, 'A'),
            'α'..='ω' => off(0x1D6FC, 'α'),
            'ϑ' => '\u{1D717}',
            'ϕ' => '\u{1D719}',
            'ϖ' => '\u{1D71B}',
            'ϱ' => '\u{1D71A}',
            'ϵ' => '\u{1D716}',
            _ => c,
        },
        Variant::Bold => match c {
            'a'..='z' => off(0x1D41A, 'a'),
            'A'..='Z' => off(0x1D400, 'A'),
            '0'..='9' => off(0x1D7CE, '0'),
            _ => c,
        },
        Variant::Double => match c {
            'C' => 'ℂ',
            'H' => 'ℍ',
            'N' => 'ℕ',
            'P' => 'ℙ',
            'Q' => 'ℚ',
            'R' => 'ℝ',
            'Z' => 'ℤ',
            'A'..='Z' => off(0x1D538, 'A'),
            'a'..='z' => off(0x1D552, 'a'),
            '0'..='9' => off(0x1D7D8, '0'),
            _ => c,
        },
        Variant::Script => match c {
            'B' => 'ℬ',
            'E' => 'ℰ',
            'F' => 'ℱ',
            'H' => 'ℋ',
            'I' => 'ℐ',
            'L' => 'ℒ',
            'M' => 'ℳ',
            'R' => 'ℛ',
            'A'..='Z' => off(0x1D49C, 'A'),
            _ => c,
        },
        Variant::Sans => match c {
            'A'..='Z' => off(0x1D5A0, 'A'),
            'a'..='z' => off(0x1D5BA, 'a'),
            _ => c,
        },
        Variant::Mono => match c {
            'A'..='Z' => off(0x1D670, 'A'),
            'a'..='z' => off(0x1D68A, 'a'),
            '0'..='9' => off(0x1D7F6, '0'),
            _ => c,
        },
    }
}

// MARK: 排版

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Style {
    Display,
    Text,
    Script,
    ScriptScript,
}

impl Style {
    fn factor(self) -> f32 {
        match self {
            Style::Display | Style::Text => 1.0,
            Style::Script => 0.7,
            Style::ScriptScript => 0.52,
        }
    }

    fn sup(self) -> Style {
        match self {
            Style::Display | Style::Text => Style::Script,
            _ => Style::ScriptScript,
        }
    }

    fn frac(self) -> Style {
        match self {
            Style::Display => Style::Text,
            Style::Text => Style::Script,
            _ => Style::ScriptScript,
        }
    }
}

struct Ctx {
    /// 正文字号（DIP）
    base: f32,
}

const AXIS: f32 = 0.25;

impl Ctx {
    fn size(&self, style: Style) -> f32 {
        self.base * style.factor()
    }

    fn char_box(&self, c: char, style: Style) -> MathLayout {
        let size = self.size(style);
        self.char_box_sized(c, size)
    }

    fn char_box_sized(&self, c: char, size: f32) -> MathLayout {
        match glyph(c as u32) {
            Some(g) => MathLayout {
                width: g.advance * size,
                ascent: g.top.max(0.0) * size,
                descent: g.bottom.max(0.0) * size,
                cmds: vec![Cmd::Glyph { x: 0.0, y: 0.0, size, index: g.index, scale_y: 1.0 }],
            },
            None => {
                let s = c.to_string();
                let width = text::measure(&s, Font::ui(size, 400));
                MathLayout { width, ascent: size * 0.75, descent: size * 0.22, cmds: vec![Cmd::Text { x: 0.0, y: 0.0, size, text: s }] }
            }
        }
    }

    fn string_box(&self, s: &str, variant: Variant, style: Style) -> MathLayout {
        let mut out = MathLayout::empty();
        for c in s.chars() {
            let b = self.char_box(map_char(c, variant), style);
            let x = out.width;
            out.place(b, x, 0.0);
        }
        out
    }

    fn text_box(&self, s: &str, style: Style) -> MathLayout {
        let size = self.size(style);
        let width = text::measure(s, Font::ui(size, 400));
        MathLayout { width, ascent: size * 0.78, descent: size * 0.24, cmds: vec![Cmd::Text { x: 0.0, y: 0.0, size, text: s.to_string() }] }
    }

    /// 一串节点横着排，节点之间按种类加间距
    fn list(&self, nodes: &[Node], style: Style) -> MathLayout {
        let mut out = MathLayout::empty();
        let mut prev: Option<Class> = None;
        let em = self.size(style);
        for node in nodes {
            let mut class = node.class();
            // 开头、左括号、运算符后面的减号是负号
            if class == Class::Bin && matches!(prev, None | Some(Class::Bin | Class::Open | Class::Rel | Class::Punct | Class::Op)) {
                class = Class::Ord;
            }
            if let Some(p) = prev {
                out.width += spacing(p, class, style) * em;
            }
            let b = self.node(node, style);
            let x = out.width;
            out.place(b, x, 0.0);
            if !matches!(node, Node::Space(_)) {
                prev = Some(class);
            }
        }
        // 结尾的二元运算符其实是普通符号，前面不该加太多空，这里不做回退，影响很小
        out
    }

    fn node(&self, node: &Node, style: Style) -> MathLayout {
        let em = self.size(style);
        match node {
            Node::Sym(c, _) => {
                let c = if style >= Style::Script { *c } else { *c };
                self.char_box(map_char(c, if ('α'..='ω').contains(&c) || matches!(c, 'ϑ' | 'ϕ' | 'ϖ' | 'ϱ' | 'ϵ') { Variant::Italic } else { Variant::Normal }), style)
            }
            Node::Chars(s, variant) => self.string_box(s, *variant, style),
            Node::Group(nodes) => self.list(nodes, style),
            Node::Text(s) => self.text_box(s, style),
            Node::Raw(s) => self.string_box(s, Variant::Normal, style),
            Node::Space(w) => MathLayout { width: w * em, ..MathLayout::empty() },
            Node::Func(name, _) => self.string_box(name, Variant::Normal, style),
            Node::BigOp(c, _) => {
                let size = if style == Style::Display { em * 1.45 } else { em * 1.08 };
                let mut b = self.char_box_sized(*c, size);
                // 大运算符居中在数学轴上
                let shift = (b.ascent - b.descent) / 2.0 - AXIS * em;
                b = b.shifted(0.0, shift);
                MathLayout { width: b.width, ascent: b.ascent - shift, descent: b.descent + shift, cmds: b.cmds }
            }
            Node::Frac { num, den, bar, size } => {
                let inner = match size {
                    Some(true) => Style::Text,
                    Some(false) => Style::Script,
                    None => style.frac(),
                };
                let n = self.node(num, inner);
                let d = self.node(den, inner);
                let thick = (0.05 * em).max(0.9);
                let gap = if style == Style::Display { 0.16 * em } else { 0.1 * em };
                let pad = 0.12 * em;
                let width = n.width.max(d.width) + pad * 2.0;
                let axis = AXIS * em;
                let mut out = MathLayout { width, ..MathLayout::empty() };
                let num_shift = axis + thick / 2.0 + gap + n.descent;
                let den_shift = -axis + thick / 2.0 + gap + d.ascent;
                out.place(n.clone(), (width - n.width) / 2.0, -num_shift);
                out.place(d.clone(), (width - d.width) / 2.0, den_shift);
                if *bar {
                    out.cmds.push(Cmd::Rule { x: pad * 0.5, y: -axis - thick / 2.0, w: width - pad, h: thick });
                }
                out.ascent = out.ascent.max(axis + thick);
                out
            }
            Node::Sqrt { index, body } => {
                let b = self.node(body, style);
                let gap = 0.12 * em;
                let thick = (0.05 * em).max(0.9);
                let top = b.ascent + gap + thick;
                let bottom = b.descent + 0.05 * em;
                let sign_w = 0.55 * em + (b.height() * 0.06);
                let mut out = MathLayout::empty();
                let mut x0 = 0.0;
                if let Some(index) = index {
                    let i = self.node(index, Style::ScriptScript);
                    out.place(i.clone(), 0.0, -(top * 0.45 + i.descent));
                    x0 = (i.width - sign_w * 0.45).max(0.0);
                }
                let h = top + bottom;
                let points = vec![
                    (x0, -h * 0.42 + bottom),
                    (x0 + sign_w * 0.22, -h * 0.5 + bottom),
                    (x0 + sign_w * 0.5, bottom),
                    (x0 + sign_w, -top + thick / 2.0),
                    (x0 + sign_w + b.width + 0.08 * em, -top + thick / 2.0),
                ];
                out.cmds.push(Cmd::Path { points, width: thick });
                out.place(b, x0 + sign_w, 0.0);
                out.ascent = out.ascent.max(top);
                out.descent = out.descent.max(bottom);
                out.width += 0.08 * em;
                out
            }
            Node::Scripts { base, sup, sub } => {
                let limits = match base.as_ref() {
                    Node::BigOp(_, l) | Node::Func(_, l) => *l && style == Style::Display,
                    _ => false,
                };
                let b = self.node(base, style);
                let script_style = style.sup();
                let sup_box = sup.as_ref().map(|s| self.node(s, script_style));
                let sub_box = sub.as_ref().map(|s| self.node(s, script_style));
                if limits {
                    let width = b.width.max(sup_box.as_ref().map_or(0.0, |s| s.width)).max(sub_box.as_ref().map_or(0.0, |s| s.width));
                    let mut out = MathLayout { width, ..MathLayout::empty() };
                    let gap = 0.12 * em;
                    out.place(b.clone(), (width - b.width) / 2.0, 0.0);
                    if let Some(s) = sup_box {
                        out.place(s.clone(), (width - s.width) / 2.0, -(b.ascent + gap + s.descent));
                    }
                    if let Some(s) = sub_box {
                        out.place(s.clone(), (width - s.width) / 2.0, b.descent + gap + s.ascent);
                    }
                    return out;
                }
                let mut out = MathLayout::empty();
                let base_w = b.width;
                let base_ascent = b.ascent;
                let base_descent = b.descent;
                let is_char = matches!(base.as_ref(), Node::Chars(..) | Node::Sym(..));
                out.place(b, 0.0, 0.0);
                let script_em = self.size(script_style);
                let mut sup_shift = if is_char { (0.42 * em).max(base_ascent - 0.25 * script_em) } else { base_ascent - 0.3 * script_em };
                let mut sub_shift = if is_char { 0.2 * em } else { (base_descent + 0.2 * script_em).max(0.2 * em) };
                if let (Some(sp), Some(sb)) = (&sup_box, &sub_box) {
                    // 上下标之间至少留一点空
                    let gap = (sup_shift - sp.descent) - (sb.ascent - sub_shift);
                    let min_gap = 0.18 * em;
                    if gap < min_gap {
                        let fix = (min_gap - gap) / 2.0;
                        sup_shift += fix;
                        sub_shift += fix;
                    }
                }
                let x = base_w + 0.03 * em;
                let mut width = x;
                if let Some(s) = sup_box {
                    width = width.max(x + s.width);
                    out.place(s, x, -sup_shift);
                }
                if let Some(s) = sub_box {
                    width = width.max(x + s.width);
                    out.place(s, x, sub_shift);
                }
                out.width = width + 0.04 * em;
                out
            }
            Node::LeftRight { left, body, right } => {
                let b = self.list(body, style);
                let axis = AXIS * em;
                let half = (b.ascent - axis).max(b.descent + axis).max(0.5 * em) + 0.08 * em;
                let mut out = MathLayout::empty();
                if let Some(c) = left {
                    let d = self.delim(*c, half * 2.0, axis, em);
                    out.place(d, 0.0, 0.0);
                }
                let x = out.width + 0.05 * em;
                out.place(b, x, 0.0);
                if let Some(c) = right {
                    let x = out.width + 0.05 * em;
                    let d = self.delim(*c, half * 2.0, axis, em);
                    out.place(d, x, 0.0);
                }
                out
            }
            Node::Big { delim, scale, .. } => {
                let axis = AXIS * em;
                self.delim(*delim, em * scale * 1.2, axis, em)
            }
            Node::Accent { kind, body } => {
                let b = self.node(body, style);
                let mut out = MathLayout::empty();
                let w = b.width;
                let top = b.ascent.max(0.45 * em) + 0.08 * em;
                let lw = (0.05 * em).max(0.9);
                let (x0, x1) = if w < 0.9 * em { (w * 0.15, w * 0.85) } else { (0.05 * em, w - 0.05 * em) };
                let mid = (x0 + x1) / 2.0;
                let h = 0.16 * em;
                out.place(b.clone(), 0.0, 0.0);
                match kind {
                    Accent::Hat => out.cmds.push(Cmd::Path { points: vec![(x0, -top), (mid, -top - h), (x1, -top)], width: lw }),
                    Accent::Check => out.cmds.push(Cmd::Path { points: vec![(x0, -top - h), (mid, -top), (x1, -top - h)], width: lw }),
                    Accent::Bar => out.cmds.push(Cmd::Rule { x: x0.min(0.02 * em), y: -top - lw, w: (x1 - x0).max(w - 0.04 * em), h: lw }),
                    Accent::Underline => out.cmds.push(Cmd::Rule { x: 0.0, y: b.descent + 0.08 * em, w, h: lw }),
                    Accent::Vec => {
                        let y = -top - h * 0.5;
                        out.cmds.push(Cmd::Path { points: vec![(x0, y), (x1, y)], width: lw });
                        out.cmds.push(Cmd::Path { points: vec![(x1 - h * 0.7, y - h * 0.6), (x1, y), (x1 - h * 0.7, y + h * 0.6)], width: lw });
                    }
                    Accent::Tilde => {
                        let pts: Vec<(f32, f32)> = (0..=12).map(|i| {
                            let t = i as f32 / 12.0;
                            (x0 + (x1 - x0) * t, -top - h * 0.5 - (t * std::f32::consts::TAU).sin() * h * 0.4)
                        }).collect();
                        out.cmds.push(Cmd::Path { points: pts, width: lw });
                    }
                    Accent::Breve => {
                        let pts: Vec<(f32, f32)> = (0..=10).map(|i| {
                            let t = i as f32 / 10.0;
                            (x0 + (x1 - x0) * t, -top - h + (t * std::f32::consts::PI).sin() * h * 0.8)
                        }).collect();
                        out.cmds.push(Cmd::Path { points: pts, width: lw });
                    }
                    Accent::Dot => out.cmds.push(Cmd::Dot { x: mid, y: -top - h * 0.4, r: lw * 1.1 }),
                    Accent::Ddot => {
                        out.cmds.push(Cmd::Dot { x: mid - h * 0.6, y: -top - h * 0.4, r: lw * 1.1 });
                        out.cmds.push(Cmd::Dot { x: mid + h * 0.6, y: -top - h * 0.4, r: lw * 1.1 });
                    }
                }
                if *kind != Accent::Underline {
                    out.ascent = out.ascent.max(top + h + lw);
                } else {
                    out.descent = out.descent.max(b.descent + 0.08 * em + lw);
                }
                out
            }
            Node::Over { over, body } => {
                let b = self.node(body, style);
                let o = self.node(over, style.sup());
                let width = b.width.max(o.width);
                let mut out = MathLayout { width, ..MathLayout::empty() };
                let ba = b.ascent;
                out.place(b.clone(), (width - b.width) / 2.0, 0.0);
                out.place(o.clone(), (width - o.width) / 2.0, -(ba + 0.1 * em + o.descent));
                out
            }
            Node::Under { under, body } => {
                let b = self.node(body, style);
                let u = self.node(under, style.sup());
                let width = b.width.max(u.width);
                let mut out = MathLayout { width, ..MathLayout::empty() };
                let bd = b.descent;
                out.place(b.clone(), (width - b.width) / 2.0, 0.0);
                out.place(u.clone(), (width - u.width) / 2.0, bd + 0.1 * em + u.ascent);
                out
            }
            Node::Boxed(body) => {
                let b = self.node(body, style);
                let pad = 0.25 * em;
                let lw = (0.05 * em).max(0.9);
                let mut out = MathLayout::empty();
                let (w, a, d) = (b.width, b.ascent, b.descent);
                out.place(b, pad, 0.0);
                let r = (0.0, -a - pad, w + pad * 2.0, a + d + pad * 2.0);
                out.cmds.push(Cmd::Path {
                    points: vec![(r.0, r.1), (r.0 + r.2, r.1), (r.0 + r.2, r.1 + r.3), (r.0, r.1 + r.3), (r.0, r.1)],
                    width: lw,
                });
                out.width = w + pad * 2.0;
                out.ascent = a + pad + lw;
                out.descent = d + pad + lw;
                out
            }
            Node::Matrix { rows, kind } => self.matrix(rows, *kind, style),
        }
    }

    /// 拉伸到 `height` 的括号，居中在数学轴上
    fn delim(&self, c: char, height: f32, axis: f32, em: f32) -> MathLayout {
        let c = match c {
            '‖' => '‖',
            other => other,
        };
        let Some(g) = glyph(c as u32) else { return self.char_box_sized(c, em) };
        let ink = (g.top + g.bottom).max(0.1);
        let size = em;
        let natural = ink * size;
        let scale_y = (height / natural).max(1.0);
        // 太高时字号也放大一点，免得括号细得像一根线
        let size = if scale_y > 1.8 { size * 1.25 } else { size };
        let natural = ink * size;
        let scale_y = (height / natural).max(1.0);
        let center = (g.top - g.bottom) / 2.0 * size;
        // 字形缩放以基线为中心，先算缩放后墨迹中心的位置，再挪到数学轴
        let y = -(axis) + center * scale_y;
        let half = natural * scale_y / 2.0;
        MathLayout {
            width: g.advance * size,
            ascent: axis + half,
            descent: (half - axis).max(0.0),
            cmds: vec![Cmd::Glyph { x: 0.0, y, size, index: g.index, scale_y }],
        }
    }

    fn matrix(&self, rows: &[Vec<Node>], kind: MatrixKind, style: Style) -> MathLayout {
        let em = self.size(style);
        let cell_style = if style == Style::Display { Style::Text } else { style };
        let cells: Vec<Vec<MathLayout>> = rows.iter().map(|r| r.iter().map(|c| self.node(c, cell_style)).collect()).collect();
        let cols = cells.iter().map(Vec::len).max().unwrap_or(0);
        let mut widths = vec![0.0f32; cols];
        for row in &cells {
            for (i, c) in row.iter().enumerate() {
                widths[i] = widths[i].max(c.width);
            }
        }
        let row_gap = 0.35 * em;
        let col_gap = match kind {
            MatrixKind::Aligned => 0.0,
            MatrixKind::Cases => 1.0 * em,
            _ => 0.9 * em,
        };
        let mut body = MathLayout::empty();
        let mut y = 0.0;
        for (ri, row) in cells.iter().enumerate() {
            let asc = row.iter().map(|c| c.ascent).fold(0.6 * em, f32::max);
            let desc = row.iter().map(|c| c.descent).fold(0.25 * em, f32::max);
            if ri > 0 {
                y += asc;
            }
            let mut x = 0.0;
            for (ci, cell) in row.iter().enumerate() {
                let w = widths[ci];
                let dx = match kind {
                    // aligned：奇数列靠右、偶数列靠左（对齐在 & 处）
                    MatrixKind::Aligned => if ci % 2 == 0 { w - cell.width } else { 0.0 },
                    MatrixKind::Cases => 0.0,
                    _ => (w - cell.width) / 2.0,
                };
                body.place(cell.clone(), x + dx, y);
                x += w + col_gap;
            }
            if ri == 0 {
                body.ascent = body.ascent.max(asc);
            }
            y += desc + row_gap;
        }
        // 整体居中在数学轴上
        let total = body.ascent + body.descent;
        let shift = body.descent - total / 2.0 + AXIS * em;
        let body = {
            let b = body.shifted(0.0, -shift);
            MathLayout { width: b.width, ascent: total / 2.0 + AXIS * em, descent: total / 2.0 - AXIS * em, cmds: b.cmds }
        };
        let (left, right) = match kind {
            MatrixKind::Paren => (Some('('), Some(')')),
            MatrixKind::Bracket => (Some('['), Some(']')),
            MatrixKind::Brace => (Some('{'), Some('}')),
            MatrixKind::Vert => (Some('|'), Some('|')),
            MatrixKind::DoubleVert => (Some('‖'), Some('‖')),
            MatrixKind::Cases => (Some('{'), None),
            _ => (None, None),
        };
        if left.is_none() && right.is_none() {
            return body;
        }
        let axis = AXIS * em;
        let half = (body.ascent - axis).max(body.descent + axis) + 0.1 * em;
        let mut out = MathLayout::empty();
        if let Some(c) = left {
            out.place(self.delim(c, half * 2.0, axis, em), 0.0, 0.0);
        }
        let x = out.width + 0.12 * em;
        out.place(body, x, 0.0);
        if let Some(c) = right {
            let x = out.width + 0.12 * em;
            out.place(self.delim(c, half * 2.0, axis, em), x, 0.0);
        }
        out
    }
}

/// 原子之间的间距（em），参照 TeX 的表简化
fn spacing(left: Class, right: Class, style: Style) -> f32 {
    use Class::*;
    let script = style >= Style::Script;
    let thin = 3.0 / 18.0;
    let med = 4.0 / 18.0;
    let thick = 5.0 / 18.0;
    match (left, right) {
        (Bin, _) | (_, Bin) if !script => med,
        (Rel, Rel) => 0.0,
        (Rel, _) | (_, Rel) if !script => thick,
        (Op, Ord) | (Ord, Op) | (Op, Op) | (Close, Op) | (Op, Inner) | (Inner, Op) => thin,
        (Punct, _) if !script => thin,
        (Inner, Ord) | (Ord, Inner) | (Inner, Inner) | (Close, Inner) | (Inner, Open) if !script => thin,
        _ => 0.0,
    }
}

thread_local! {
    static CACHE: RefCell<HashMap<(String, u32, bool), Option<MathLayout>>> = RefCell::new(HashMap::new());
}

/// 排一个公式。`size` 是正文字号，`display` 是独立成行的大号公式。没有 Cambria Math 时返回 None
pub fn layout(latex: &str, size: f32, display: bool) -> Option<MathLayout> {
    let key = (latex.to_string(), (size * 10.0) as u32, display);
    if let Some(hit) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return hit;
    }
    let result = if available() {
        let tokens = tokenize(latex);
        let mut parser = Parser { tokens, pos: 0 };
        let mut nodes = Vec::new();
        // 多余的 } 跳过，继续解析
        while parser.pos < parser.tokens.len() {
            nodes.extend(parser.expr());
            parser.pos += 1;
        }
        let ctx = Ctx { base: size };
        let style = if display { Style::Display } else { Style::Text };
        Some(ctx.list(&nodes, style))
    } else {
        None
    };
    CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        if cache.len() > 400 {
            cache.clear();
        }
        cache.insert(key, result.clone());
    });
    result
}

/// 没有公式字体时的退路：显示原文
pub fn source_layout(latex: &str, size: f32) -> MathLayout {
    let width = text::measure(latex, Font::ui(size, 400));
    MathLayout { width, ascent: size * 0.78, descent: size * 0.24, cmds: vec![Cmd::Text { x: 0.0, y: 0.0, size, text: latex.to_string() }] }
}

/// 没有公式字体时的退路：原文用等宽字体
pub fn fallback_width(latex: &str, size: f32) -> f32 {
    text::measure(latex, Font::mono(size * 0.9)) + 8.0
}

#[allow(dead_code)]
pub fn interpolation() -> D2D1_BITMAP_INTERPOLATION_MODE {
    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR
}
