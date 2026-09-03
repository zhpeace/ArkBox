use std::sync::Arc;

use crate::archive;
use crate::edit;
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
    archive::extract(&archive, &dest, entries.as_deref(), password.as_deref(), cb)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_entries(archive: String, password: Option<String>) -> Result<Vec<EntryInfo>, String> {
    archive::list_entries(&archive, password.as_deref()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn preview_entry(
    archive: String,
    entry: String,
    password: Option<String>,
    max_bytes: Option<usize>,
) -> Result<PreviewData, String> {
    let max = max_bytes.unwrap_or(1_000_000);
    archive::preview_entry(&archive, &entry, password.as_deref(), max).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn test_archive(archive: String, password: Option<String>) -> Result<TestResult, String> {
    archive::test_archive(&archive, password.as_deref()).map_err(|e| e.to_string())
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

/// 取走冷启动时通过文件关联传入的待打开压缩包路径（取后清空）
#[tauri::command]
pub fn take_pending_open(state: State<PendingOpen>) -> Vec<String> {
    let mut g = state.0.lock().unwrap();
    std::mem::take(&mut *g)
}
