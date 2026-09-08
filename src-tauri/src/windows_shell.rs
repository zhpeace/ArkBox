//! Windows 右键上下文菜单注册（HKCU，用户级，无需管理员权限）。
//!
//! 该模块仅在 `cfg(windows)` 下被 `lib.rs` 声明编译（见 `#[cfg(windows)] mod windows_shell;`），
//! macOS / Linux 完全不引入 `winreg` 也不编译本文件。
//!
//! 行为：app 启动时调用 [`register_context_menus`]，向注册表写入「用 ArkBox 解压 / 用 ArkBox 压缩」。
//! - 文件（*）右键：同时显示「解压」（打开浏览）与「压缩」（进入压缩页预填）
//! - 目录（Directory）右键：显示「压缩」
//! 命令调用 `arkbox.exe "%1"`（解压）或 `arkbox.exe --compress "%1"`（压缩）。
//!
//! 卸载后残留的注册表项无害（指向 arkbox.exe，删除 app 后点击会报"找不到文件"，
//! 后续可在 uninstall 阶段清理，当前不做以免影响其他用户配置）。

use std::path::PathBuf;
use winreg::enums::*;
use winreg::RegKey;

const VERB_EXTRACT: &str = "用 ArkBox 解压";
const VERB_COMPRESS: &str = "用 ArkBox 压缩";

/// 取 arkbox.exe 自身路径（含空格需外层加引号，由调用方处理）。
fn exe_path() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// 在 `base\shell\<verb>` 下写入显示名与 command。
fn set_verb(base: &str, verb_key: &str, label: &str, command: &str) -> std::io::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(&format!("{}\\shell\\{}", base, verb_key))?;
    key.set_value("", &label)?; // 默认值是菜单显示文字
    if let Some(exe) = exe_path() {
        if let Some(s) = exe.to_str() {
            // 菜单项图标用 app 自身 exe（Windows 会取其内嵌图标）
            let _ = key.set_value("Icon", &format!("\"{}\"", s));
        }
    }
    let (cmd, _) = hkcu.create_subkey(&format!("{}\\shell\\{}\\command", base, verb_key))?;
    cmd.set_value("", &command)?;
    Ok(())
}

/// 注册文件与目录的右键上下文菜单。失败时仅打印告警，不阻塞 app 启动。
pub fn register_context_menus() {
    let exe = match exe_path() {
        Some(e) if e.to_str().is_some() => e,
        _ => {
            eprintln!("[windows] 无法获取 arkbox.exe 路径，跳过右键菜单注册");
            return;
        }
    };
    // 路径含空格必须用引号包裹；%1 是 Windows 右键传入的选中文件路径，同样需引号。
    let exe_quoted = format!("\"{}\"", exe.to_str().unwrap());
    let extract_cmd = format!("{} \"%1\"", exe_quoted);
    let compress_cmd = format!("{} --compress \"%1\"", exe_quoted);

    // 文件（所有类型）：解压 + 压缩
    if let Err(e) = set_verb("Software\\Classes\\*", "ArkBoxExtract", VERB_EXTRACT, &extract_cmd) {
        eprintln!("[windows] 注册「解压」失败: {}", e);
    }
    if let Err(e) = set_verb("Software\\Classes\\*", "ArkBoxCompress", VERB_COMPRESS, &compress_cmd) {
        eprintln!("[windows] 注册「压缩」失败: {}", e);
    }
    // 目录：仅压缩
    if let Err(e) =
        set_verb("Software\\Classes\\Directory", "ArkBoxCompress", VERB_COMPRESS, &compress_cmd)
    {
        eprintln!("[windows] 注册目录「压缩」失败: {}", e);
    }

    println!("[windows] 右键上下文菜单已注册（解压 / 压缩）");
}
