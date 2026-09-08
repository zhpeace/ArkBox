use std::sync::Arc;

use crate::archive;
use crate::edit;
use crate::keychain::{load_archive_password, save_archive_password};
use crate::PendingOpen;
use crate::model::{ArchiveInfo, EditHandle, EntryInfo, ExtractResult, PreviewData, PresetConfig, ProgressPayload, ProgressFn, TestResult};
use crate::presets;
use tauri::{AppHandle, Emitter, State};

#[tauri::command]
pub fn compress(
    app: AppHandle,
    paths: Vec<String>,
    dest: String,
    format: String,
    password: Option<String>,
    level: u8,
    exclude: Vec<String>,
    compress_hidden: bool,
) -> Result<ArchiveInfo, String> {
    let cb: Option<ProgressFn> = Some(Arc::new({
        let app = app.clone();
        move |current, total| {
            let _ = app.emit(
                "archive-progress",
                ProgressPayload {
                    kind: "compress".into(),
                    current,
                    total,
                },
            );
        }
    }));
    let info = archive::compress(
        &paths,
        &dest,
        &format,
        password.as_deref(),
        level,
        &exclude,
        compress_hidden,
        cb,
    )
    .map_err(|e| e.to_string())?;

    // 记住加密密码，便于后续浏览 / 解压时自动填充
    if let Some(pw) = password.as_deref() {
        let _ = save_archive_password(&dest, pw);
    }

    // 压缩后自动跑完整性检测（用同一密码），把结果推给前端
    if let Ok(t) = archive::test_archive(&dest, password.as_deref()) {
        let _ = app.emit("archive-verified", t);
    }
    Ok(info)
}

#[tauri::command]
pub fn extract(
    app: AppHandle,
    archive: String,
    dest: String,
    entries: Option<Vec<String>>,
    password: Option<String>,
) -> Result<ExtractResult, String> {
    let cb: Option<ProgressFn> = Some(Arc::new({
        let app = app.clone();
        move |current, total| {
            let _ = app.emit(
                "archive-progress",
                ProgressPayload {
                    kind: "extract".into(),
                    current,
                    total,
                },
            );
        }
    }));
    let resolved = password.or_else(|| load_archive_password(&archive));
    let pw = resolved.as_deref();
    archive::extract(&archive, &dest, entries.as_deref(), pw, cb)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_entries(archive: String, password: Option<String>) -> Result<Vec<EntryInfo>, String> {
    let resolved = password.or_else(|| load_archive_password(&archive));
    let pw = resolved.as_deref();
    archive::list_entries(&archive, pw).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn preview_entry(
    archive: String,
    entry: String,
    password: Option<String>,
    max_bytes: Option<usize>,
) -> Result<PreviewData, String> {
    let max = max_bytes.unwrap_or(1_000_000);
    let resolved = password.or_else(|| load_archive_password(&archive));
    let pw = resolved.as_deref();
    archive::preview_entry(&archive, &entry, pw, max).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn test_archive(archive: String, password: Option<String>) -> Result<TestResult, String> {
    let resolved = password.or_else(|| load_archive_password(&archive));
    let pw = resolved.as_deref();
    archive::test_archive(&archive, pw).map_err(|e| e.to_string())
}

/// 提取归档内某个条目（通常为嵌套压缩包）到临时文件并返回其路径，
/// 供前端以该临时文件为 archive 实现逐层浏览（方案 A）。
#[tauri::command]
pub fn extract_nested(archive: String, entry: String, password: Option<String>) -> Result<String, String> {
    let resolved = password.or_else(|| load_archive_password(&archive));
    let pw = resolved.as_deref();
    archive::extract_one_to_temp(&archive, &entry, pw)
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_preset(cfg: PresetConfig) -> Result<(), String> {
    presets::save_preset(&cfg).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn load_presets() -> Result<Vec<PresetConfig>, String> {
    presets::load_presets().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_preset(name: String) -> Result<(), String> {
    presets::delete_preset(&name).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn begin_edit_entry(
    archive: String,
    entry: String,
    password: Option<String>,
) -> Result<EditHandle, String> {
    edit::begin_edit_entry(&archive, &entry, password.as_deref()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn commit_edit(handle: EditHandle, archive: String, password: Option<String>) -> Result<(), String> {
    edit::commit_edit(&handle, &archive, password.as_deref()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn cancel_edit(handle: EditHandle) -> Result<(), String> {
    edit::cancel_edit(handle).map_err(|e| e.to_string())
}

// 档案内增删/重命名条目：复用 edit 的「全量解包 -> 改 -> 重打包」路径。
// 密码统一走钥匙串解析（与 extract/list_entries 同款写法）。

#[tauri::command]
pub fn delete_entries(
    archive: String,
    entries: Vec<String>,
    password: Option<String>,
) -> Result<(), String> {
    let resolved = password.or_else(|| load_archive_password(&archive));
    let pw = resolved.as_deref();
    edit::delete_entries(&archive, &entries, pw).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn rename_entry(
    archive: String,
    old_name: String,
    new_name: String,
    password: Option<String>,
) -> Result<(), String> {
    let resolved = password.or_else(|| load_archive_password(&archive));
    let pw = resolved.as_deref();
    edit::rename_entry(&archive, &old_name, &new_name, pw).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn add_entries(
    archive: String,
    new_files: Vec<String>,
    password: Option<String>,
) -> Result<(), String> {
    let resolved = password.or_else(|| load_archive_password(&archive));
    let pw = resolved.as_deref();
    edit::add_entries(&archive, &new_files, pw).map_err(|e| e.to_string())
}

/// 取走冷启动时通过文件关联传入的待打开压缩包路径（取后清空）
#[tauri::command]
pub fn take_pending_open(state: State<PendingOpen>) -> Vec<String> {
    let mut g = state.0.lock().unwrap();
    std::mem::take(&mut *g)
}

/// 在文件管理器中定位并选中给定文件/目录
/// （跨平台：macOS `open -R` / Windows `explorer /select,` / Linux `xdg-open`）
#[tauri::command]
pub fn reveal_in_finder(path: String) -> Result<(), String> {
    use std::process::Command;
    #[cfg(target_os = "macos")]
    let (bin, args): (&str, Vec<String>) = ("open", vec!["-R".to_string(), path.clone()]);
    #[cfg(target_os = "windows")]
    let (bin, args): (&str, Vec<String>) = ("explorer", vec![format!("/select,{}", path)]);
    #[cfg(target_os = "linux")]
    let (bin, args): (&str, Vec<String>) = ("xdg-open", vec![path.clone()]);
    let status = Command::new(bin)
        .args(&args)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("{bin} 退出码: {}", status.code().unwrap_or(-1)));
    }
    Ok(())
}

/// 用系统默认程序打开本地文件（编辑压缩包内条目时打开临时副本）。
/// 不走 @tauri-apps/plugin-shell 的 open：它按 URL 正则校验（仅放行 http/https/mailto/tel），
/// 传本地路径会报 "failed regex validation"。
/// 跨平台：macOS `open` / Windows `explorer` / Linux `xdg-open`。
#[tauri::command]
pub fn open_with_default_app(path: String) -> Result<(), String> {
    use std::process::Command;
    #[cfg(target_os = "macos")]
    let (bin, args): (&str, Vec<String>) = ("open", vec![path.clone()]);
    #[cfg(target_os = "windows")]
    let (bin, args): (&str, Vec<String>) = ("explorer", vec![path.clone()]);
    #[cfg(target_os = "linux")]
    let (bin, args): (&str, Vec<String>) = ("xdg-open", vec![path.clone()]);
    let status = Command::new(bin)
        .args(&args)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!(
            "无法用默认程序打开 {path}（{bin} 退出码: {}）",
            status.code().unwrap_or(-1)
        ));
    }
    Ok(())
}
