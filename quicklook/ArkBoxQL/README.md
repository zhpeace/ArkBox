# ArkBoxQL —— Quick Look 预览插件

在 Finder 里选中压缩包、**按空格**即可看到包内文件清单，无需打开 app。这是 ArkBox 最招牌的系统集成能力。

## 架构

```
ArkBoxQL.qlgenerator/
└── Contents/
    ├── Info.plist          # CFPlugIn 工厂 + 声明可预览的 UTI
    └── MacOS/
        ├── ArkBoxQL     # C 端 QL 生成器（spawn helper + 喂 HTML 给 Quick Look）
        └── bz-qlhelper     # Rust 辅助二进制（复用归档逻辑，渲染自包含 HTML 到 stdout）
```

- 压缩/解压/列目录的全部逻辑都在 Rust（`src-tauri/src/bin/qlhelper.rs`），C 端只做管道。
- 预览回调：`GeneratePreviewForURL` 调同目录 `bz-qlhelper <archive>`，把 HTML 通过
  `QLPreviewRequestSetDataRepresentation(kUTTypeHTML)` 交给 Quick Look 渲染。
- 缩略图回调：`GenerateThumbnailForURL` 画一张带格式标签（zip/7z/tar/gz）的卡片。

## 构建

```bash
bash build.sh
```

产物：`ArkBoxQL.qlgenerator`（arm64，Apple Silicon）。

> Intel Mac 需改 build.sh：clang 加 `-arch x86_64`，helper 用
> `cargo build --target x86_64-apple-darwin --bin bz-qlhelper`，再用 `lipo` 合并两份可执行。

## 安装（开发/本机）

```bash
cp -r ArkBoxQL.qlgenerator ~/Library/QuickLook/
qlmanage -r            # 刷新 Quick Look 生成器缓存
```

## 测试

```bash
qlmanage -p /path/to/demo.zip     # 弹出预览窗口（需 GUI 会话）
qlmanage -t -s 512 -o ~/Desktop /path/to/demo.zip   # 仅生成缩略图到 png
```

 Finder 里选中任意 zip/7z/tar(.gz/.bz2/.xz) 按空格即可看到内容清单。

## 卸载

```bash
rm -rf ~/Library/QuickLook/ArkBoxQL.qlgenerator
qlmanage -r
```

## 打包进主 app（可选后续）

`npm run tauri build` 后可把本 `.qlgenerator` 拷进
`ArkBox.app/Contents/Library/QuickLook/` 随 app 分发，使安装 app 即获得预览能力。
需在 Tauri 的 build 后处理脚本里加一步拷贝（路径随 Tauri 打包布局而定）。
