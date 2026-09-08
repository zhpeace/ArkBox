//! 用打包内置的 7z 二进制（mac: `7zz`, win: `7z.exe`）兜底处理 BetterZip 支持、但 Rust 引擎未覆盖的只读格式：
//! iso / cab / cpio / dmg / deb / rpm / xar / pkg / brotli(.br) / 纯 tar 等。
//!
//! 策略：仅做「列出 / 解压 / 预览 / 完整性校验」四类只读操作；创建仍走 Rust 引擎
//! （zip / sevenz / tar / single 等）。二进制路径在 `lib.rs` 启动时经 `init_bin` 写入全局 `BIN`。
//! 选择 7z 而非纯 Rust crate，是因为 p7zip 的 `7z` 一个二进制即原生支持上述全部格式，
//! 且跨平台自包含（mac 用官方 7zz 同款 p7zip 二进制，win 用 7z.exe + 7z.dll）。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use base64::Engine;
use tauri::AppHandle;
use tauri::Manager;

use crate::error::{AppError, AppResult};
use crate::model::{EntryInfo, ExtractResult, PreviewData, ProgressFn, TestResult};

static BIN: OnceLock<PathBuf> = OnceLock::new();

/// 由 `lib.rs` 在 app 启动后调用，解析内置 7z 二进制绝对路径并缓存。
pub fn init_bin(app: &AppHandle) {
    let bin_name = if cfg!(target_os = "windows") {
        "7z.exe"
    } else {
        "7zz"
    };
    if let Some(dir) = app.path().resource_dir().ok() {
        let p = dir.join("binaries").join(bin_name);
        if p.exists() {
            println!("[7z] 引擎就绪: {}", p.display());
            let _ = BIN.set(p);
        } else {
            eprintln!("[7z] 未找到内置二进制: {}", p.display());
        }
    } else {
        eprintln!("[7z] 无法解析 resource_dir，跳过 7z 引擎初始化");
    }
}

pub fn bin_path() -> Option<&'static PathBuf> {
    BIN.get()
}

fn require_bin() -> AppResult<&'static PathBuf> {
    bin_path().ok_or_else(|| {
        AppError::UnsupportedFormat("7z 引擎未初始化（内置 7z 二进制缺失，iso/cab/dmg 等格式不可用）".into())
    })
}

/// 列出归档内条目（7z l -slt，逐块解析 Path/Size/Attributes/Packed Size/Method）。
pub fn list_entries(archive: &str, password: Option<&str>) -> AppResult<Vec<EntryInfo>> {
    let bin = require_bin()?;
    let mut cmd = Command::new(bin.as_os_str());
    cmd.arg("l").arg("-slt").arg(archive);
    if let Some(pw) = password.filter(|s| !s.is_empty()) {
        cmd.arg(format!("-p{}", pw));
    }
    let out = cmd
        .output()
        .map_err(|e| AppError::Other(format!("调用 7z 失败: {e}")))?;
    if !out.status.success() {
        return Err(AppError::UnsupportedFormat(format!(
            "7z 列出失败: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    parse_list(&String::from_utf8_lossy(&out.stdout))
}

fn parse_list(s: &str) -> AppResult<Vec<EntryInfo>> {
    let mut entries: Vec<EntryInfo> = Vec::new();
    let mut cur: HashMap<&str, String> = HashMap::new();
    let flush = |cur: &mut HashMap<&str, String>, entries: &mut Vec<EntryInfo>| {
        if let Some(path) = cur.get("Path") {
            if path.is_empty() {
                return;
            }
            let is_dir = cur
                .get("Attributes")
                .map(|a| a.contains('D'))
                .unwrap_or(false);
            let size = cur.get("Size").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
            let compressed = cur.get("Packed Size").and_then(|v| v.parse::<u64>().ok());
            let method = cur.get("Method").cloned();
            entries.push(EntryInfo {
                name: path.clone(),
                size,
                compressed_size: compressed,
                is_dir,
                modified: None,
                method,
            });
        }
    };
    for line in s.lines() {
        let line = line.trim();
        if line.is_empty() {
            flush(&mut cur, &mut entries);
            cur.clear();
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            cur.insert(k.trim(), v.trim().to_string());
        }
    }
    flush(&mut cur, &mut entries);
    Ok(entries)
}

/// 解压（7z x，保留目录结构）。entries 为 None 解压全部。
pub fn extract(
    archive: &str,
    dest: &str,
    entries: Option<&[String]>,
    password: Option<&str>,
    on_progress: Option<ProgressFn>,
) -> AppResult<ExtractResult> {
    let bin = require_bin()?;
    fs::create_dir_all(dest).map_err(AppError::Io)?;
    let mut cmd = Command::new(bin.as_os_str());
    cmd.arg("x").arg(archive).arg("-o").arg(dest).arg("-y");
    if let Some(es) = entries {
        for e in es {
            cmd.arg(e);
        }
    }
    if let Some(pw) = password.filter(|s| !s.is_empty()) {
        cmd.arg(format!("-p{}", pw));
    }
    let out = cmd
        .output()
        .map_err(|e| AppError::Other(format!("调用 7z 失败: {e}")))?;
    if !out.status.success() {
        return Err(AppError::Archive(format!(
            "7z 解压失败: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    let count = count_files(Path::new(dest));
    if let Some(cb) = on_progress {
        cb(count as u64, count.max(1) as u64);
    }
    Ok(ExtractResult {
        extracted: count,
        dest: dest.to_string(),
    })
}

fn count_files(dir: &Path) -> usize {
    let mut n = 0;
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                n += count_files(&p);
            } else {
                n += 1;
            }
        }
    }
    n
}

/// 预览单个条目：提取到临时目录后读内容（7z e 不保留路径，按 basename 读）。
pub fn preview_entry(
    archive: &str,
    entry: &str,
    password: Option<&str>,
    max_bytes: usize,
) -> AppResult<PreviewData> {
    let bin = require_bin()?;
    let tmp = tempfile::tempdir().map_err(AppError::Io)?;
    let basename = Path::new(entry)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| entry.replace(['/', '\\'], "_"));
    let mut cmd = Command::new(bin.as_os_str());
    cmd.arg("e")
        .arg(archive)
        .arg(entry)
        .arg(format!("-o{}", tmp.path().display()))
        .arg("-y");
    if let Some(pw) = password.filter(|s| !s.is_empty()) {
        cmd.arg(format!("-p{}", pw));
    }
    let out = cmd
        .output()
        .map_err(|e| AppError::Other(format!("调用 7z 失败: {e}")))?;
    if !out.status.success() {
        return Err(AppError::Archive(format!(
            "7z 提取条目失败: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    let extracted = tmp.path().join(&basename);
    if !extracted.exists() {
        return Err(AppError::UnsupportedFormat(format!("归档内找不到条目: {entry}")));
    }
    let data = fs::read(&extracted).map_err(AppError::Io)?;
    build_preview(entry, &data, max_bytes)
}

fn build_preview(name: &str, data: &[u8], max: usize) -> AppResult<PreviewData> {
    let truncated = data.len() > max;
    let head = if truncated { &data[..max] } else { data };
    // 含 NUL 字节或含 UTF-8 替换字符，判为二进制
    let looks_binary = head.contains(&0)
        || std::str::from_utf8(head)
            .map(|s| s.contains('\u{fffd}'))
            .unwrap_or(true);
    if looks_binary {
        Ok(PreviewData {
            name: name.to_string(),
            mime: mime_of(name),
            text: None,
            data_base64: Some(base64::engine::general_purpose::STANDARD.encode(head)),
            truncated,
            is_binary: true,
        })
    } else {
        Ok(PreviewData {
            name: name.to_string(),
            mime: "text/plain".to_string(),
            text: Some(String::from_utf8_lossy(head).to_string()),
            data_base64: None,
            truncated,
            is_binary: false,
        })
    }
}

fn mime_of(name: &str) -> String {
    let ext = Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "xml" => "application/xml",
        "html" | "htm" => "text/html",
        "md" => "text/markdown",
        "css" => "text/css",
        "js" => "application/javascript",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// 完整性校验（7z t）。以 "Everything is Ok" 判定通过。
pub fn test_archive(archive: &str, password: Option<&str>) -> AppResult<TestResult> {
    let bin = require_bin()?;
    let mut cmd = Command::new(bin.as_os_str());
    cmd.arg("t").arg(archive);
    if let Some(pw) = password.filter(|s| !s.is_empty()) {
        cmd.arg(format!("-p{}", pw));
    }
    let out = cmd
        .output()
        .map_err(|e| AppError::Other(format!("调用 7z 失败: {e}")))?;
    let s = String::from_utf8_lossy(&out.stdout);
    let ok = out.status.success() && s.contains("Everything is Ok");
    Ok(TestResult {
        ok,
        entries: vec![],
    })
}
