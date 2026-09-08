# ArkBox · 方舟压包

一款 macOS 平台的压缩/解压工具，对标 [BetterZip](https://macitbetter.com/)。基于 **Tauri 2 + Rust + Vue 3** 构建，采用「开发者签名 · 非沙盒」方案，可直接读写任意位置的文件。

> 当前版本 `0.1.0`，处于早期阶段。核心压缩/解压能力已具备，UI 与系统集成仍在打磨。

---

## 功能特性

- **批量压缩**：拖入多个文件/文件夹，一键打包为 zip / 7z / tar 系格式
- **密码压缩**：zip 与 7z 支持 AES-256 加密（7z 同时加密文件头），密码自动记入 macOS 钥匙串，下次浏览 / 解压免重复输入
- **浏览即解压**：打开压缩包后直接查看目录树，按需部分解压、单文件预览，**无需先整体解压**
- **内置预览**：文本 / 图片 / 二进制十六进制预览，不落地即可看内容
- **完整性校验**：解压前对压缩包做自校验（CRC / 解密试读），失败直接提示
- **macOS 元数据保真（zip）**：压缩 zip 时保留 unix 权限、符号链接与扩展属性（xattr，经 `__MACOSX`/AppleDouble），解压完整还原；7z 受上游限制暂不支持（见已知问题）
- **归档内编辑**：在不解压的情况下修改包内某个文件并写回（编辑 → 提交 → 取消）
- **档案内增删 / 重命名条目**：浏览界面可删除单个/批量条目、对条目（含目录）重命名、向包内添加本地文件，均通过「全量解包 → 改 → 重打包」实现，加密包沿用原密码（RAR 为只读格式，不支持）
- **压缩预设**：把常用的格式 / 密码 / 排除规则存为预设，下次一键复用
- **系统集成**
  - 文件关联：双击 `.zip` / `.7z` / `.rar` 等直接打开
  - 拖放：把文件拖进窗口即可压缩，把压缩包拖进窗口即可解压
  - **Quick Look**：在 Finder 里按空格预览压缩包内容（独立 Quick Look 插件）
  - **Finder Sync**：右键上下文菜单快速调用
  - NSServices：从其他 App 的「服务」菜单把文件送进 ArkBox

---

## 格式支持矩阵

| 格式 | 创建（压缩） | 解压 / 浏览 | 加密 | 备注 |
|------|:---:|:---:|:---:|------|
| `.zip` | ✅ | ✅ | AES-256 | 列表/预览/校验路径须正确传密码（见已知问题） |
| `.7z` | ✅ | ✅ | AES-256 + 加密文件头 | `num_cycles_power = 19` 对齐 7-Zip 官方 |
| `.tar` | ✅ | ✅ | — | |
| `.tar.gz` / `.tgz` | ✅ | ✅ | — | |
| `.tar.bz2` / `.tbz` | ✅ | ✅ | — | |
| `.tar.xz` / `.txz` | ✅ | ✅ | — | |
| `.tar.zst` / `.tzst` | ✅ | ✅ | — | |
| `.gz` / `.bz2` / `.xz` / `.zst` | ✅（单流） | ✅ | — | 仅压缩单文件流，不含打包 |
| `.rar` | ❌ | ✅（只读） | 支持列/解/预览/校验 | **只能读不能写**，见下方许可说明 |
| `.zipx` | ❌ | — | — | 别名识别为 zip，暂不支持其专属算法 |

> RAR 写入被**有意禁止**：RAR 编码器为 win.rar GmbH 专有，从未开源授权，任何开源项目都不得创建 RAR（BetterZip 同样是只读）。

---

## 构建与运行

### 环境要求

- **Rust** 工具链（stable）
- **Node.js** ≥ 18 与 npm
- **Xcode Command Line Tools**：`unrar` crate 会静态链接官方 UnRAR C++ 源码，需要 `clang++`；Quick Look / Finder Sync 扩展也依赖它
- macOS 11+（Swift 扩展需要）

### 步骤

```bash
# 1. 安装前端依赖
npm install

# 2. 开发模式（同时起 Vite + Rust 侧 watch）
npm run dev

# 3. 正式构建并打包 DMG（含 Quick Look / Finder Sync 扩展注入）
npm run tauri:build
# 等价于：
#   tauri build
#   && bash quicklook/bundle-into-app.sh   # 把 .qlgenerator 打进 .app
#   && bash scripts/make-dmg.sh            # 产出 .dmg
```

构建产物：`src-tauri/target/release/bundle/macos/ArkBox.app` 与 `src-tauri/target/release/bundle/dmg/*.dmg`（均已被 `.gitignore` 忽略）。

### 单独构建扩展

```bash
bash quicklook/ArkBoxQL/build.sh      # 产出 ArkBoxQL.qlgenerator
bash findersync/build.sh              # 产出 ArkBoxFSE.appex
```

---

## 已知问题 / 限制

1. **7z 加密文件头已加密，但 sevenz-rust 解密头不校验密码（上游缺陷）**
   当前 sevenz-rust 0.6.1 在加密时会加密文件头，已用「单文件 + 极短文件名」回归测试覆盖：**无密码连文件名都列不出**。但上游在解密文件头时不校验密码，实测**任意非空密码都能列出文件名与大小**，只有真正取内容时才会因密码错误而失败。即 7z 加密保护数据内容、不保护「文件名枚举」——数据本身仍安全。该缺陷无法从调用侧绕过，只能在使用说明里讲清楚。

2. **Bundle ID 为占位**
   `tauri.conf.json` 中 `identifier = com.arkbox.desktop` 是占位值（并未持有 `arkbox` 域名）。正式分发与公证前需替换为你自己的 reverse-DNS ID，并配置对应的 **Developer ID** 证书与**公证**流程（当前本地构建为 ad-hoc 签名）。

3. **`cargo test --release` 不可直接跑**
   发布配置设了 `panic = "abort"`，与集成测试要求的 unwind 策略冲突。日常用 `cargo test`（dev profile）即可；若确需 release 下跑测试，可临时 `cargo test --release --config 'profile.release.panic="unwind"'`。

4. **7z 不保真 macOS 元数据**
   `sevenz-rust` 0.6.1 的 `SevenZArchiveEntry` 没有 posix 权限 / xattr / 符号链接字段，无法在 7z 中保留 unix 权限与扩展属性。仅 zip 支持元数据保真（见功能特性）；若需保留元数据，请用 zip 格式打包。

5. **单流格式（`.gz` / `.bz2` / `.xz` / `.zst`）不支持「添加条目」**
   这些格式只容纳单个文件，重命名 / 删除可用，「添加文件」会被拒绝（添加多个条目无意义）。多文件归档请用 zip / 7z / tar 系格式。另：增删/重命名走「全量解包 → 重打包」，会沿用原压缩等级（默认 6）与加密密码，但 7z 重打包后同样不保真 macOS 元数据（见第 4 条）。

---

## 许可

- 本项目主体以 **MIT** 许可发布（见 `LICENSE`）。
- **RAR 解压能力**基于 UnRAR 源码（由 `unrar` crate 静态链接），其许可要求：**可用于解压，但不得用于创建 RAR 文件，且必须保留 UnRAR 许可声明**。本项目严格遵守，不提供任何 RAR 创建功能。`LICENSE` 文件已逐字收录 UnRAR 许可第 2 条。
