pub mod preview;
pub mod rar;
pub mod sevenz;
pub mod single;
pub mod tarball;
pub mod test;
pub mod zip;

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};
use crate::model::{ArchiveFormat, ArchiveInfo, EntryInfo, ExtractResult, PreviewData, ProgressFn, TestResult};

/// 判断某个路径是否应被排除（压缩时）。
pub fn should_exclude(
    rel_name: &str,
    exclude: &[String],
    compress_hidden: bool,
) -> bool {
    if !compress_hidden {
        // 以 '.' 开头的文件/目录（Unix 隐藏文件）以及 macOS 资源叉
        let leaf = Path::new(rel_name)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if leaf.starts_with('.') {
            return true;
        }
    }
    let lower = rel_name.to_ascii_lowercase();
    for pat in exclude {
        let p = pat.trim();
        if p.is_empty() {
            continue;
        }
        if lower.ends_with(p.to_ascii_lowercase().as_str()) {
            return true;
        }
        // 支持简单的目录前缀排除（如 "node_modules/"）
        if lower == p.to_ascii_lowercase().as_str()
            || lower.starts_with(&format!("{}/", p.to_ascii_lowercase()))
        {
            return true;
        }
    }
    false
}

fn detect_format(archive: &str) -> AppResult<ArchiveFormat> {
    ArchiveFormat::from_ext(archive)
        .ok_or_else(|| AppError::UnsupportedFormat(format!("无法识别压缩格式: {archive}")))
}

/// 压缩一组路径到目标文件。
pub fn compress(
    paths: &[String],
    dest: &str,
    format: &str,
    password: Option<&str>,
    level: u8,
    exclude: &[String],
    compress_hidden: bool,
    on_progress: Option<ProgressFn>,
) -> AppResult<ArchiveInfo> {
    let fmt = ArchiveFormat::from_str(format)
        .ok_or_else(|| AppError::UnsupportedFormat(format!("不支持的格式: {format}")))?;

    match fmt {
        ArchiveFormat::Zip => {
            zip::compress_zip(paths, dest, password, level, exclude, compress_hidden, on_progress.as_ref())
        }
        ArchiveFormat::SevenZip => {
            sevenz::compress_7z(paths, dest, password, level, exclude, compress_hidden, on_progress.as_ref())
        }
        ArchiveFormat::Gz | ArchiveFormat::Bz2 | ArchiveFormat::Xz | ArchiveFormat::Zstd => {
            single::compress_single(paths, dest, fmt, level, on_progress.as_ref())
        }
        ArchiveFormat::Rar => Err(AppError::UnsupportedFormat(
            "RAR 是 win.rar 的专有格式，编码器从未开源授权，无法创建 RAR；请选择 zip 或 7z".into(),
        )),
        _ => tarball::compress_tar(paths, dest, fmt, level, exclude, compress_hidden, on_progress.as_ref()),
    }
}

/// 解压。entries 为 None 时解压全部。
pub fn extract(
    archive: &str,
    dest: &str,
    entries: Option<&[String]>,
    password: Option<&str>,
    on_progress: Option<ProgressFn>,
) -> AppResult<ExtractResult> {
    let fmt = detect_format(archive)?;
    match fmt {
        ArchiveFormat::Zip => zip::extract_zip(archive, dest, entries, password, on_progress.as_ref()),
        ArchiveFormat::SevenZip => sevenz::extract_7z(archive, dest, entries, password, on_progress.as_ref()),
        ArchiveFormat::Gz | ArchiveFormat::Bz2 | ArchiveFormat::Xz | ArchiveFormat::Zstd => {
            single::extract_single(archive, dest, fmt, on_progress.as_ref())
        }
        ArchiveFormat::Rar => rar::extract_rar(archive, dest, entries, password, on_progress.as_ref()),
        _ => tarball::extract_tar(archive, dest, entries, on_progress.as_ref()),
    }
}

pub fn list_entries(archive: &str, password: Option<&str>) -> AppResult<Vec<EntryInfo>> {
    let fmt = detect_format(archive)?;
    match fmt {
        ArchiveFormat::Zip => zip::list_zip(archive, password),
        ArchiveFormat::SevenZip => sevenz::list_7z(archive, password),
        ArchiveFormat::Gz | ArchiveFormat::Bz2 | ArchiveFormat::Xz | ArchiveFormat::Zstd => {
            single::list_single(archive, fmt)
        }
        ArchiveFormat::Rar => rar::list_rar(archive, password),
        _ => tarball::list_tar(archive),
    }
}

pub fn preview_entry(
    archive: &str,
    entry: &str,
    password: Option<&str>,
    max_bytes: usize,
) -> AppResult<PreviewData> {
    let fmt = detect_format(archive)?;
    match fmt {
        ArchiveFormat::Zip => preview::preview_zip(archive, entry, password, max_bytes),
        ArchiveFormat::SevenZip => preview::preview_7z(archive, entry, password, max_bytes),
        ArchiveFormat::Gz | ArchiveFormat::Bz2 | ArchiveFormat::Xz | ArchiveFormat::Zstd => {
            single::preview_single(archive, fmt, max_bytes)
        }
        ArchiveFormat::Rar => rar::preview_rar(archive, entry, password, max_bytes),
        _ => preview::preview_tar(archive, entry, max_bytes),
    }
}

pub fn test_archive(archive: &str, password: Option<&str>) -> AppResult<TestResult> {
    let fmt = detect_format(archive)?;
    match fmt {
        ArchiveFormat::Zip => test::test_zip(archive, password),
        ArchiveFormat::SevenZip => test::test_7z(archive, password),
        ArchiveFormat::Gz | ArchiveFormat::Bz2 | ArchiveFormat::Xz | ArchiveFormat::Zstd => {
            single::test_single(archive, fmt)
        }
        ArchiveFormat::Rar => rar::test_rar(archive, password),
        _ => test::test_tar(archive),
    }
}

/// 预扫描待压缩输入，统计会被加入的文件数（与 add_* 的排除/隐藏规则一致），用作进度条分母。
pub fn count_input_files(paths: &[String], exclude: &[String], compress_hidden: bool) -> usize {
    let mut n = 0usize;
    for p in paths {
        let abs = PathBuf::from(p);
        let base = abs
            .parent()
            .map(|x| x.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        collect_count(&abs, &base, exclude, compress_hidden, &mut n);
    }
    n
}

fn collect_count(abs: &Path, base: &Path, exclude: &[String], compress_hidden: bool, n: &mut usize) {
    let rel = abs
        .strip_prefix(base)
        .unwrap_or(abs)
        .to_string_lossy()
        .replace('\\', "/");
    if rel.is_empty() {
        return;
    }
    if should_exclude(&rel, exclude, compress_hidden) {
        return;
    }
    if abs.is_dir() {
        if let Ok(rd) = fs::read_dir(abs) {
            for e in rd.flatten() {
                collect_count(&e.path(), base, exclude, compress_hidden, n);
            }
        }
    } else {
        *n += 1;
    }
}
