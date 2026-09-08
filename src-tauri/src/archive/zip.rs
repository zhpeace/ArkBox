use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{copy, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use base64::Engine;
use zip::read::ZipFile;
use zip::write::{ExtendedFileOptions, FileOptions};
use zip::{AesMode, CompressionMethod, ZipArchive, ZipWriter};

use crate::apple_double;
use crate::archive::should_exclude;
use crate::error::{AppError, AppResult};
use crate::model::{
    ArchiveInfo, EntryInfo, EntryTestStatus, ExtractResult, PreviewData, ProgressFn, TestResult,
};

fn clamp_level(level: u8) -> i64 {
    level.clamp(0, 9) as i64
}

fn base_opts<'k>(level: u8, password: Option<&'k str>) -> FileOptions<'k, ExtendedFileOptions> {
    let mut opts = FileOptions::<'k, ExtendedFileOptions>::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(clamp_level(level)));
    if let Some(pw) = password {
        if !pw.is_empty() {
            opts = opts.with_aes_encryption(AesMode::Aes256, pw);
        }
    }
    opts
}

fn open_entry<'a>(
    za: &'a mut ZipArchive<fs::File>,
    i: usize,
    password: Option<&str>,
) -> Result<ZipFile<'a>, zip::result::ZipError> {
    match password {
        Some(pw) if !pw.is_empty() => za.by_index_decrypt(i, pw.as_bytes()),
        _ => za.by_index(i),
    }
}

/// 把一批输入路径压缩成 zip。
pub fn compress_zip(
    paths: &[String],
    dest: &str,
    password: Option<&str>,
    level: u8,
    exclude: &[String],
    compress_hidden: bool,
    on_progress: Option<&ProgressFn>,
) -> AppResult<ArchiveInfo> {
    if paths.is_empty() {
        return Err(AppError::Archive("未提供任何待压缩路径".into()));
    }
    let file = fs::File::create(dest)
        .map_err(|e| AppError::Io(std::io::Error::new(e.kind(), format!("{dest}: {e}"))))?;
    let mut zw = ZipWriter::new(file);

    let total = crate::archive::count_input_files(paths, exclude, compress_hidden);
    let mut done = 0usize;

    for p in paths {
        let abs = PathBuf::from(p);
        if !abs.exists() {
            return Err(AppError::Archive(format!("路径不存在: {p}")));
        }
        let base = abs
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        add_path(
            &mut zw,
            &abs,
            &base,
            password,
            level,
            exclude,
            compress_hidden,
            &mut done,
            total,
            on_progress,
        )?;
    }

    zw.finish()
        .map_err(|e| AppError::Archive(format!("写入 zip 失败: {e}")))?;

    summarize(dest, "zip")
}

fn add_path(
    zw: &mut ZipWriter<fs::File>,
    abs: &Path,
    base: &Path,
    password: Option<&str>,
    level: u8,
    exclude: &[String],
    compress_hidden: bool,
    done: &mut usize,
    total: usize,
    on_progress: Option<&ProgressFn>,
) -> AppResult<()> {
    let rel = abs.strip_prefix(base).unwrap_or(abs);
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    if rel_str.is_empty() {
        return Ok(());
    }
    if should_exclude(&rel_str, exclude, compress_hidden) {
        return Ok(());
    }

    // 符号链接：不跟随，记录目标路径（unix 模式标记为 0o120777）
    if abs.is_symlink() {
        let target = fs::read_link(abs)?;
        let target_str = target.to_string_lossy().to_string();
        let opts = base_opts(level, password).unix_permissions(0o777);
        zw.add_symlink(&rel_str, &target_str, opts)
            .map_err(|e| AppError::Archive(e.to_string()))?;
        *done += 1;
        maybe_write_apple_double(zw, abs, &rel_str, password, level)?;
        if let Some(cb) = on_progress {
            cb(*done as u64, total as u64);
        }
        return Ok(());
    }

    if abs.is_dir() {
        let mode = dir_mode(abs)?;
        let opts = base_opts(level, password).unix_permissions(mode);
        zw.add_directory(format!("{rel_str}/"), opts)
            .map_err(|e| AppError::Archive(e.to_string()))?;
        maybe_write_apple_double(zw, abs, &rel_str, password, level)?;
        for entry in fs::read_dir(abs)? {
            let entry = entry?;
            add_path(
                zw,
                &entry.path(),
                base,
                password,
                level,
                exclude,
                compress_hidden,
                done,
                total,
                on_progress,
            )?;
        }
    } else {
        let mode = file_mode(abs)?;
        let opts = base_opts(level, password).unix_permissions(mode);
        zw.start_file(&rel_str, opts)
            .map_err(|e| AppError::Archive(e.to_string()))?;
        let mut f = fs::File::open(abs)?;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            zw.write_all(&buf[..n])
                .map_err(|e| AppError::Archive(e.to_string()))?;
        }
        *done += 1;
        maybe_write_apple_double(zw, abs, &rel_str, password, level)?;
        if let Some(cb) = on_progress {
            cb(*done as u64, total as u64);
        }
    }
    Ok(())
}

pub fn extract_zip(
    archive: &str,
    dest: &str,
    entries: Option<&[String]>,
    password: Option<&str>,
    on_progress: Option<&ProgressFn>,
) -> AppResult<ExtractResult> {
    let file = fs::File::open(archive)?;
    let mut za = ZipArchive::new(file)
        .map_err(|e| AppError::Archive(format!("无法打开 zip: {e}")))?;
    let wanted: Option<HashSet<String>> = entries.map(|v| v.iter().cloned().collect());
    fs::create_dir_all(dest)?;
    let mut count = 0usize;

    // 第一遍：收集 __MACOSX 元数据；统计真实文件体积
    let mut xattr_map: BTreeMap<String, BTreeMap<String, Vec<u8>>> = BTreeMap::new();
    let mut total = 0u64;
    for i in 0..za.len() {
        let mut f = open_entry(&mut za, i, password)
            .map_err(|e| AppError::Archive(e.to_string()))?;
        let name = f.name().to_string();
        if let Some(real) = strip_apple_double(&name) {
            let mut buf = Vec::new();
            f.read_to_end(&mut buf)?;
            if let Some((_fi, others)) = apple_double::parse(&buf) {
                xattr_map.insert(real, others);
            }
            continue;
        }
        if f.is_dir() || name.ends_with('/') {
            continue;
        }
        if let Some(set) = &wanted {
            if !set.contains(&name) {
                continue;
            }
        }
        total += f.size();
    }

    let mut current = 0u64;
    for i in 0..za.len() {
        let mut f = open_entry(&mut za, i, password)
            .map_err(|e| AppError::Archive(e.to_string()))?;
        let name = f.name().to_string();
        if name.starts_with("__MACOSX/") {
            continue;
        }
        if let Some(set) = &wanted {
            if !set.contains(&name) {
                continue;
            }
        }
        let out_path = Path::new(dest).join(sanitize(&name));
        if f.is_dir() || name.ends_with('/') {
            fs::create_dir_all(&out_path)?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)?;
        }
        #[cfg(unix)]
        {
            let mode = f.unix_mode().unwrap_or(0o644);
            if mode & 0o170000 == 0o120000 {
                // 符号链接：内容即目标路径
                let mut target = String::new();
                f.read_to_string(&mut target)?;
                let _ = fs::remove_file(&out_path);
                std::os::unix::fs::symlink(&target, &out_path)
                    .map_err(|e| AppError::Archive(format!("创建符号链接失败: {e}")))?;
            } else {
                let mut out = fs::File::create(&out_path)?;
                copy(&mut f, &mut out)?;
                let perm = PermissionsExt::from_mode(mode & 0o7777);
                fs::set_permissions(&out_path, perm)?;
            }
            if let Some(xattrs) = xattr_map.get(&name) {
                for (k, v) in xattrs {
                    let _ = xattr::set(&out_path, k, v);
                }
            }
        }
        #[cfg(windows)]
        {
            let mut out = fs::File::create(&out_path)?;
            copy(&mut f, &mut out)?;
        }
        count += 1;
        current += f.size();
        if let Some(cb) = on_progress {
            cb(current, total);
        }
    }
    Ok(ExtractResult {
        extracted: count,
        dest: dest.to_string(),
    })
}

pub fn list_zip(archive: &str, password: Option<&str>) -> AppResult<Vec<EntryInfo>> {
    let file = fs::File::open(archive)?;
    let mut za = ZipArchive::new(file)
        .map_err(|e| AppError::Archive(format!("无法打开 zip: {e}")))?;
    let mut out = Vec::with_capacity(za.len());
    for i in 0..za.len() {
        let f = open_entry(&mut za, i, password).map_err(|e| AppError::Archive(e.to_string()))?;
        let name = f.name().to_string();
        if name.starts_with("__MACOSX/") {
            continue;
        }
        out.push(EntryInfo {
            name,
            size: f.size(),
            compressed_size: Some(f.compressed_size()),
            is_dir: f.is_dir(),
            modified: None,
            method: Some(format!("{:?}", f.compression())),
        });
    }
    Ok(out)
}

pub fn preview_zip(
    archive: &str,
    entry: &str,
    password: Option<&str>,
    max_bytes: usize,
) -> AppResult<PreviewData> {
    let file = fs::File::open(archive)?;
    let mut za = ZipArchive::new(file)
        .map_err(|e| AppError::Archive(format!("无法打开 zip: {e}")))?;
    for i in 0..za.len() {
        let mut f = open_entry(&mut za, i, password)
            .map_err(|e| AppError::Archive(e.to_string()))?;
        let name = f.name().to_string();
        if name != entry {
            continue;
        }
        if f.is_dir() {
            return Err(AppError::Archive("该条目是目录，无法预览".into()));
        }
        return read_preview(&mut f, entry, max_bytes);
    }
    Err(AppError::Archive(format!("未找到条目: {entry}")))
}

pub fn test_zip(archive: &str, password: Option<&str>) -> AppResult<TestResult> {
    let file = fs::File::open(archive)?;
    let mut za = ZipArchive::new(file)
        .map_err(|e| AppError::Archive(format!("无法打开 zip: {e}")))?;
    let mut entries_out = Vec::with_capacity(za.len());
    let mut all_ok = true;
    for i in 0..za.len() {
        let mut f = open_entry(&mut za, i, password)
            .map_err(|e| AppError::Archive(e.to_string()))?;
        let name = f.name().to_string();
        if name.ends_with('/') {
            continue;
        }
        match copy(&mut f, &mut std::io::sink()) {
            Ok(_) => entries_out.push(EntryTestStatus {
                name,
                ok: true,
                error: None,
            }),
            Err(e) => {
                all_ok = false;
                entries_out.push(EntryTestStatus {
                    name,
                    ok: false,
                    error: Some(e.to_string()),
                });
            }
        }
    }
    Ok(TestResult {
        ok: all_ok,
        entries: entries_out,
    })
}

pub(crate) fn read_preview<R: Read>(r: &mut R, name: &str, max_bytes: usize) -> AppResult<PreviewData> {
    let mut buf = Vec::with_capacity(max_bytes.min(1024 * 1024));
    let mut chunk = [0u8; 8192];
    let mut truncated = false;
    loop {
        if buf.len() >= max_bytes {
            truncated = true;
            break;
        }
        let to_read = (max_bytes - buf.len()).min(chunk.len());
        let n = r.read(&mut chunk[..to_read])?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let is_binary = looks_binary(&buf);
    let mime = guess_mime(name, is_binary);
    if is_binary {
        Ok(PreviewData {
            name: name.to_string(),
            mime,
            text: None,
            data_base64: Some(base64::engine::general_purpose::STANDARD.encode(&buf)),
            truncated,
            is_binary: true,
        })
    } else {
        Ok(PreviewData {
            name: name.to_string(),
            mime,
            text: Some(String::from_utf8_lossy(&buf).to_string()),
            data_base64: None,
            truncated,
            is_binary: false,
        })
    }
}

pub(crate) fn looks_binary(buf: &[u8]) -> bool {
    buf.iter().take(8192).any(|&b| b == 0)
}

pub(crate) fn guess_mime(name: &str, is_binary: bool) -> String {
    if !is_binary {
        return "text/plain".into();
    }
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".png") {
        "image/png".into()
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg".into()
    } else if lower.ends_with(".gif") {
        "image/gif".into()
    } else if lower.ends_with(".webp") {
        "image/webp".into()
    } else {
        "application/octet-stream".into()
    }
}

/// 防止路径穿越：去掉绝对前缀与 `..`。
pub(crate) fn sanitize(name: &str) -> PathBuf {
    let cleaned: String = name
        .split('/')
        .filter(|seg| !(*seg == "." || *seg == ".." || seg.is_empty()))
        .collect::<Vec<_>>()
        .join("/");
    PathBuf::from(cleaned)
}

/// 真实文件 unix 权限（含普通文件类型位 0o100000）。仅 unix 平台解析真实权限。
#[cfg(unix)]
fn file_mode(abs: &Path) -> AppResult<u32> {
    let m = abs.metadata()?.mode();
    Ok((m & 0o7777) | 0o100000)
}

#[cfg(windows)]
fn file_mode(_abs: &Path) -> AppResult<u32> {
    Ok(0o100644)
}

/// 目录 unix 权限（含目录类型位 0o040000）。仅 unix 平台解析真实权限。
#[cfg(unix)]
fn dir_mode(abs: &Path) -> AppResult<u32> {
    let m = abs.metadata()?.mode();
    Ok((m & 0o7777) | 0o040000)
}

#[cfg(windows)]
fn dir_mode(_abs: &Path) -> AppResult<u32> {
    Ok(0o040644)
}

/// 收集文件/目录的全部扩展属性（仅 macOS 有意义；其他平台不写 AppleDouble，返回空）
#[cfg(target_os = "macos")]
fn collect_xattrs(abs: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut map = BTreeMap::new();
    if let Ok(names) = xattr::list(abs) {
        for name in names {
            if let Ok(Some(val)) = xattr::get(abs, &name) {
                map.insert(name.to_string_lossy().to_string(), val);
            }
        }
    }
    map
}

#[cfg(not(target_os = "macos"))]
fn collect_xattrs(_abs: &Path) -> BTreeMap<String, Vec<u8>> {
    BTreeMap::new()
}

/// 真实相对路径 -> `__MACOSX` 下的 AppleDouble 条目名（仅 macOS 写入时用到）
#[cfg(target_os = "macos")]
fn apple_double_entry_name(rel_str: &str) -> String {
    let idx = rel_str.rfind('/').map(|i| i + 1).unwrap_or(0);
    let (dir, name) = rel_str.split_at(idx);
    format!("__MACOSX/{}._{}", dir.trim_end_matches('/'), name)
}

/// 若文件带扩展属性，写一个 `__MACOSX` AppleDouble 条目保真元数据（仅 macOS）
#[cfg(target_os = "macos")]
fn maybe_write_apple_double(
    zw: &mut ZipWriter<fs::File>,
    abs: &Path,
    rel_str: &str,
    password: Option<&str>,
    level: u8,
) -> AppResult<()> {
    let xattrs = collect_xattrs(abs);
    if xattrs.is_empty() {
        return Ok(());
    }
    let mut finder_info = [0u8; 32];
    if let Some(fi) = xattrs.get("com.apple.FinderInfo") {
        let n = fi.len().min(32);
        finder_info[..n].copy_from_slice(&fi[..n]);
    }
    let mut others = xattrs;
    others.remove("com.apple.FinderInfo");
    let blob = apple_double::build(&finder_info, &others);
    let ad_name = apple_double_entry_name(rel_str);
    let opts = base_opts(level, password);
    zw.start_file(&ad_name, opts)
        .map_err(|e| AppError::Archive(e.to_string()))?;
    zw.write_all(&blob).map_err(|e| AppError::Archive(e.to_string()))?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn maybe_write_apple_double(
    _zw: &mut ZipWriter<fs::File>,
    _abs: &Path,
    _rel_str: &str,
    _password: Option<&str>,
    _level: u8,
) -> AppResult<()> {
    Ok(())
}

/// `__MACOSX/<dir>/._<name>` -> 真实相对路径 `<dir>/<name>`
fn strip_apple_double(name: &str) -> Option<String> {
    if !name.starts_with("__MACOSX/") {
        return None;
    }
    let rest = &name["__MACOSX/".len()..];
    if let Some(idx) = rest.rfind('/') {
        let (dir, last) = rest.split_at(idx);
        if !last.starts_with("/._") {
            return None;
        }
        Some(format!("{}/{}", dir, &last[3..]))
    } else {
        if !rest.starts_with("._") {
            return None;
        }
        Some(rest[2..].to_string())
    }
}

fn summarize(dest: &str, format: &str) -> AppResult<ArchiveInfo> {
    let meta = fs::metadata(dest)?;
    let count = count_entries(dest)?;
    Ok(ArchiveInfo {
        path: dest.to_string(),
        format: format.to_string(),
        entry_count: count,
        total_size: meta.len(),
    })
}

fn count_entries(dest: &str) -> AppResult<usize> {
    let file = fs::File::open(dest)?;
    let za = ZipArchive::new(file)
        .map_err(|e| AppError::Archive(format!("无法打开 zip: {e}")))?;
    Ok(za.len())
}
