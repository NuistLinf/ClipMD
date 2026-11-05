# SwiftMath（内置副本）

- 来源：https://github.com/mgriebling/SwiftMath
- 版本：1.7.3（提交 `fa8244ed032f4a1ade4cb0571bf87d2f1a9fd2d7`）
- 许可：MIT，见 `LICENSE`；Latin Modern Math 字体用的是 GUST Font License，见 `mathFonts.bundle/GUST-FONT-LICENSE.txt`

## 为什么拷贝源码，而不是用 SwiftPM 依赖

SwiftPM 给资源生成的 `Bundle.module` 只会去 `.app` 的根目录找资源包，但 macOS 的代码签名不允许根目录有额外内容（`unsealed contents present in the bundle root`）。所以把源码放进来，改成自己指定字体的位置。

## 改了什么

1. 新增 `Sources/SwiftMath/MathBundle/MathFontResources.swift`：先找下载好的字体 `~/Library/Application Support/ClipMD/mathFonts.bundle`，再找 App 的 `Contents/Resources`
2. `MathBundle/MathFont.swift` 的两处、`MathRender/MTFont.swift` 的一处，把 `Bundle.module.url(forResource: "mathFonts", withExtension: "bundle")` 换成 `MathFontResources.bundleURL`（每处都留了注释）
3. 不带任何字体。ClipMD 只用 Latin Modern Math（默认字体），由用户在预览里点了才下载（`Sources/ClipMD/MathFontInstaller.swift`，固定提交、校验 SHA-256），约 0.8 MB；原仓库 12 种字体一共 7.1 MB
4. 没有带原仓库的测试和示例

其他源码文件和原版一致。升级时重新拷贝 `Sources/SwiftMath` 下的 `MathBundle`、`MathRender`，再按上面的第 2 条改一遍。
