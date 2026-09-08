# ArkBox 跨平台右键（上下文菜单）注册说明

`bz-cli` 是 ArkBox 的 headless 静默压缩/解压辅助二进制，支持三种触发：
- **macOS Finder 服务（Services）**：已在 `Info.plist.extra` 声明，由 LaunchServices 注册；右键文件 → 服务 → 「用 ArkBox 压缩/解压」。删 app 即失效（零残留）。
- **命令行（全平台）**：`bz-cli compress <path> [path...]` / `bz-cli extract <path> [path...]`，供注册表/脚本/手动调用。
- 压缩逻辑：单文件/目录 → 同级 `<原名>.zip`；多文件 → 公共父目录名 `.zip`；解压 → 同级 `<去扩展名>` 目录。

## 把 bz-cli 打进 Windows/Linux 包（接入前提）
`bz-cli` 已作为 `[[bin]]` 存在，`beforeBuildCommand` 会在构建期预编它。
但 Tauri 默认只把**主程序**拷进包，需显式把它带进 Windows/Linux 包：
- **Windows**：用 `externalBin`（Tauri 期望预建二进制名为 `bz-cli-<target-triple>.exe`，如 `bz-cli-x86_64-pc-windows-msvc.exe`），或在 `bundle.windows.files` 里 `{"bz-cli.exe": "target/release/bz-cli.exe"}`（注意本仓库实测 Tauri 的 `files` 映射是 **value=源、key=包内目标**）。
- **Linux**：同上，`bundle.linux.files: {"bz-cli": "target/release/bz-cli"}`，并确保它落到 `/usr/bin` 或 PATH（deb 的 `files` 默认进资源目录而非 bin，可能需改 install 前缀或软链）。
- ⚠️ 以上 Windows/Linux 打包配置**未经本机（macOS）验证**，请在对应平台 `tauri build` 实测。

## Windows 注册（右键菜单）
参考 `windows/arkbox-shell.nsi`：在 NSIS 安装段写 `HKCR\*\shell\ArkBoxCompress[Extract]` 两项，
command 指向 `"$INSTDIR\bz-cli.exe" compress "%1"`。卸载段删这两项即清。
（若不用 NSIS 而用 WiX，改 `<RegistryKey>` 等价的 `<RegistryValue>` 写入。）

## Linux 注册（文件管理器右键）
- **Nemo / Caja**：把 `linux/nemo-arkbox-compress.action` 与 `nemo-arkbox-extract.action` 复制到
  `~/.local/share/nemo/actions/`（用户级）或 `/usr/share/nemo/actions/`（系统级），重启文件管理器即可。
  脚本里 `bz-cli` 需在 PATH；否则改成绝对路径（如 `/usr/bin/bz-cli`）。
- **Nautilus**：在 `~/.local/share/nautilus/scripts/` 放两个可执行脚本，内容 `bz-cli compress "$@"` / `bz-cli extract "$@"`，
  右键 → 脚本 子菜单出现。
- **Thunar**：在 `~/.config/Thunar/uca.xml` 加两条自定义动作，命令 `bz-cli compress %F` / `bz-cli extract %F`。
- 删除这些文件/配置即卸载右键项（满足「卸载即清」）。

## 验证

### macOS（本机，沙箱 headless 无法触发 AppleEvent）
1. 把 `ArkBox.app` 拖进 `/Applications`，确保 LaunchServices 扫描：
   ```bash
   /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f /Applications/ArkBox.app
   # 查服务是否已注册（应看到 arkboxCompress / arkboxExtract）
   /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -dump | grep -i arkbox
   ```
2. Finder 里选中文件/文件夹 → 右键 → 「服务」→ 「用 ArkBox 压缩」应静默生成同级 `.zip` 并用 `open -R` 定位；选中压缩包 → 「用 ArkBox 解压」→ 同级同名目录。
3. 若右键「服务」里**不出现** ArkBox 项：把 `src/bin/bz-cli.rs` 的 `setActivationPolicy: 1`（Accessory）改成 `0`（Regular）重新构建再试。
4. 卸载即清：删 `/Applications/ArkBox.app` 后 `lsregister -dump | grep arkbox` 应为空（零残留）。

### Windows（本机 `tauri build` + 安装后）
1. 安装后确认右键项注册：
   ```powershell
   reg query "HKCR\*\shell\ArkBoxCompress"
   reg query "HKCR\*\shell\ArkBoxExtract"
   ```
2. 右键任意文件 → 「用 ArkBox 压缩」；右键压缩包 → 「用 ArkBox 解压」。
3. CLI 自测（无需安装）：`target\release\bz-cli.exe compress <文件>`。
4. 卸载即清：运行卸载程序后上述注册表项应消失。

### Linux（本机 `tauri build` + 安装后）
1. 把 `linux/nemo-arkbox-compress.action` / `nemo-arkbox-extract.action` 复制到
   `~/.local/share/nemo/actions/`（Nautilus 用 `~/.local/share/nautilus/scripts/` 脚本），
   并确保 `bz-cli` 在 PATH（否则把 `.action` 里 `Exec=bz-cli ...` 改成绝对路径，如 `/usr/bin/bz-cli`）。
2. 重启文件管理器（`nemo -q` 或注销重登），右键文件 → 「用 ArkBox 压缩/解压」。
3. CLI 自测：`bz-cli compress <文件>`。
4. 卸载即清：删掉 `.action` / 脚本即消失。

> ⚠️ 三平台的「把 bz-cli 打进安装包」步骤见上方「把 bz-cli 打进 Windows/Linux 包」；
> 其中 `bundle.windows.files` / `bundle.linux.files` 的 tauri.conf.json 改动属既有配置，需你授权后再落地。
