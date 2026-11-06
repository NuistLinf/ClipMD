# ClipMD Windows 版

用 Rust 写的 Windows 版 ClipMD，功能和 macOS 版一样：托盘图标 + 悬浮窗，复制 AI 的回复一键存成 Markdown 文件、文件放回剪贴板；
截图、标注、取色、钉在桌面、识别文字；预览编辑 md、原生渲染 LaTeX 公式。

**要求**：Windows 10 1809 或更新的版本，64 位。Windows 11 22H2 以上悬浮窗背后是系统的亚克力材质，更早的版本是不透明的背景。

## 安装

到 [Releases](https://github.com/NuistLinf/ClipMD/releases) 下载 `ClipMD-x.y.z-windows-x64.zip`，解压出 `ClipMD.exe`，
放到一个固定的地方（比如 `D:\Tools\ClipMD\`）双击打开。不需要安装，没有别的文件。

- 第一次打开时 Windows 可能提示"已保护你的电脑"（程序没有付费的代码签名证书）：点「更多信息 → 仍要运行」
- 想开机自动启动：在 ClipMD 的设置里打开「开机自动启动」
- 想放进开始菜单：右键 `ClipMD.exe` →「固定到开始屏幕」
- 卸载：在设置里关掉开机启动，退出 ClipMD，删掉 `ClipMD.exe` 和 `%LOCALAPPDATA%\ClipMD` 文件夹

## 用法

| 快捷键 | 作用 |
|---|---|
| Ctrl+Alt+S | 把剪贴板存成 Markdown |
| Ctrl+Alt+A | 截图 |
| Ctrl+Alt+O | 截图并识别文字 |
| 连点两次 Ctrl+C | 直接保存（可在设置里关掉） |

截图界面里：`R` 矩形、`O` 椭圆、`M` 马赛克、`T` 识别文字、`P` 钉在桌面、`C` 复制色号、`Shift+C` 复制 rgb、`Ctrl+Z` 撤销、
回车或双击选区完成、`Esc` 或右键取消。

三个快捷键都能在设置里改。Ctrl+Alt+A 和 QQ 的截图快捷键一样，如果被占用了，ClipMD 会提示，到设置里换一个就行。

**识别文字**用的是系统自带的 Windows OCR，完全在本机。识别中文需要系统里有中文的"光学字符识别"语言功能：
中文版 Windows 一般自带；没有的话到「设置 → 时间和语言 → 语言和区域」里给中文添加。

## 数据放在哪

- 保存的 md 文件：默认 `文档\ClipMD`，可以在设置里改
- 设置、剪贴板记录、日志：`%LOCALAPPDATA%\ClipMD\`

不联网。剪贴板记录不记密码管理器复制的内容（带 `ExcludeClipboardContentFromMonitorProcessing` 等标记的）。

## 从源码编译

需要 [Rust](https://rustup.rs/) 和 Visual Studio 的「使用 C++ 的桌面开发」（要用里面的链接器和 rc.exe）：

```powershell
git clone https://github.com/NuistLinf/ClipMD.git
cd ClipMD\windows
cargo build --release
.\target\release\ClipMD.exe
```

- `cargo test -p clipmd-core`：平台无关逻辑的测试（HTML 转 Markdown、命名、公式提取……），和 macOS 版是同一套用例，在 macOS、Linux 上也能跑
- `ClipMD.exe --selftest <文件夹>`：用单独的数据文件夹把程序完整跑一遍，各个窗口截图、检查保存和截图的结果，写进 `report.txt`。
  GitHub Actions 每次提交都会在 Windows 上跑一遍（`.github/workflows/windows.yml`），截图在运行结果的 artifact 里
- 在 macOS 上可以做类型检查：`rustup target add x86_64-pc-windows-msvc && cargo check --target x86_64-pc-windows-msvc`
- `scripts/make-icon.sh`（macOS）：从 macOS 版的图标生成 `app/res/ClipMD.ico`

## 代码结构

```
windows/
├── core/          平台无关的逻辑（clipmd-core），对应 macOS 版的 ClipMDCore
│   ├── html_to_markdown.rs   网页 HTML → Markdown（html5ever 解析）
│   ├── clipboard_markdown.rs 纯文本和网页格式选哪个
│   ├── math_extractor.rs     公式换成占位符，免得 Markdown 解析器动它
│   ├── markdown.rs           预览用的块结构（pulldown-cmark）
│   ├── note_namer.rs / note_writer.rs  起名、存文件、列出最近的文件
│   ├── clip_history.rs       剪贴板记录的规则
│   └── dropped_text.rs / rtf.rs / codepage.rs  拖进来的文本文件（UTF-8、GB18030、UTF-16、RTF）
└── app/           Windows 界面（Win32 + Direct2D + DirectComposition）
    ├── gfx.rs / text.rs / icons.rs / ui.rs / anim.rs   绘制底座：交换链、画笔、文字、矢量图标、控件、弹簧动画
    ├── panel.rs              悬浮窗
    ├── app.rs                启动、托盘、快捷键、所有操作
    ├── watcher.rs / clipboard.rs  监听复制、读写剪贴板
    ├── capture/              截图：冻结屏幕、选区、放大镜、标注、导出
    ├── pin.rs / ocr.rs       钉在桌面、识别文字（子进程）
    ├── preview.rs / editor.rs / markdown_view.rs / math_render.rs  预览编辑、Markdown 排版、LaTeX 公式（Cambria Math）
    ├── clip_preview.rs / settings_window.rs / hud.rs  剪贴板预览、设置、提示
    └── selftest.rs           自检
```

为了性能，界面全部自己画：不嵌浏览器，不用跨平台界面库。闲着时没有定时器（剪贴板变化是系统通知的，动画只在动的时候才请求帧），
截图界面关掉后冻结的整屏画面全部释放，识别文字在用完就退出的子进程里做。
