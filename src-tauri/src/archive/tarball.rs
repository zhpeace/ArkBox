use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use bzip2::read::BzDecoder;
use bzip2::write::BzEncoder;
use bzip2::Compression as BzCompression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression as GzCompression;
use tar::{Archive, Builder};
use xz2::read::XzDecoder;
use xz2::write::XzEncoder;

use crate::archive::should_exclude;
use crate::archive::zip::read_preview;
use crate::error::{AppError, AppResult};
use crate::model::{
    ArchiveFormat, ArchiveInfo, EntryInfo, EntryTestStatus, ExtractResult, PreviewData, ProgressFn,
    TestResult,
};

fn tar_decoder(fmt: ArchiveFormat, file: fs::File) -> AppResult<Box<dyn Read>> {
    match fmt {
        ArchiveFormat::TarGz => Ok(Box::new(GzDecoder::new(file))),
        ArchiveFormat::TarBz => Ok(Box::new(BzDecoder::new(file))),
        ArchiveFormat::TarXz => Ok(Box::new(XzDecoder::new(file))),
        ArchiveFormat::TarZstd => Ok(Box::new(
            zstd::stream::read::Decoder::new(file)
                .map_err(|e| AppError::Archive(e.to_string()))?,
        )),
        _ => Err(AppError::UnsupportedFormat(format!(
            "tar 不支持的格式: {:?}",
            fmt
        ))),
    }
}

pub fn compress_tar(
    paths: &[String],
    dest: &str,
    fmt: ArchiveFormat,
    level: u8,
    exclude: &[String],
    compress_hidden: bool,
    on_progress: Option<&ProgressFn>,
) -> AppResult<ArchiveInfo> {
    if paths.is_empty() {
        return Err(AppError::Archive("未提供任何待压缩路径".into()));
    }
    let total = crate::archive::count_input_files(paths, exclude, compress_hidden);
    let mut done = 0usize;
    let file = fs::File::create(dest)
        .map_err(|e| AppError::Io(std::io::Error::new(e.kind(), format!("{dest}: {e}"))))?;
    match fmt {
        ArchiveFormat::TarGz => {
            let enc = GzEncoder::new(file, GzCompression::new(level.clamp(0, 9) as u32));
            let mut b = Builder::new(enc);
            add_tar_paths(&mut b, paths, exclude, compress_hidden, &mut done, total, on_progress)?;
            b.finish().map_err(io_to_app)?;
        }
        ArchiveFormat::TarBz => {
            let enc = BzEncoder::new(file, BzCompression::new(level.clamp(0, 9) as u32));
            let mut b = Builder::new(enc);
            add_tar_paths(&mut b, paths, exclude, compress_hidden, &mut done, total, on_progress)?;
            b.finish().map_err(io_to_app)?;
        }
        ArchiveFormat::TarXz => {
            let enc = XzEncoder::new(file, 6);
            let mut b = Builder::new(enc);
            add_tar_paths(&mut b, paths, exclude, compress_hidden, &mut done, total, on_progress)?;
            b.finish().map_err(io_to_app)?;
        }
        ArchiveFormat::TarZstd => {
            let mut enc = zstd::stream::write::Encoder::new(file, level.clamp(0, 22) as i32)
                .map_err(|e| AppError::Archive(e.to_string()))?;
            {
                let mut b = Builder::new(&mut enc);
                add_tar_paths(&mut b, paths, exclude, compress_hidden, &mut done, total, on_progress)?;
                b.finish().map_err(io_to_app)?;
            }
            enc.finish().map_err(io_to_app)?;
        }
        _ => return Err(AppError::UnsupportedFormat("非 tar 格式".into())),
    }
    summarize_tar(dest, fmt)
}

fn add_tar_paths<W: Write>(
    builder: &mut Builder<W>,
    paths: &[String],
    exclude: &[String],
    compress_hidden: bool,
    done: &mut usize,
    total: usize,
    on_progress: Option<&ProgressFn>,
) -> AppResult<()> {
    for p in paths {
        let abs = PathBuf::from(p);
        if !abs.exists() {
            return Err(AppError::Archive(format!("路径不存在: {p}")));
        }
        let base = abs
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        add_tar_one(builder, &abs, &base, exclude, compress_hidden, done, total, on_progress)?;
    }
    Ok(())
}

fn add_tar_one<W: Write>(
    builder: &mut Builder<W>,
    abs: &Path,
    base: &Path,
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
    if abs.is_dir() {
        builder
            .append_dir(&rel_str, abs)
            .map_err(|e| AppError::Archive(e.to_string()))?;
        for entry in fs::read_dir(abs)? {
            let entry = entry?;
            add_tar_one(builder, &entry.path(), base, exclude, compress_hidden, done, total, on_progress)?;
        }
    } else {
        builder
            .append_path_with_name(abs, &rel_str)
            .map_err(|e| AppError::Archive(e.to_string()))?;
        *done += 1;
        if let Some(cb) = on_progress {
            cb(*done as u64, total as u64);
        }
    }
    Ok(())
}

pub fn extract_tar(
    archive: &str,
    dest: &str,
    entries: Option<&[String]>,
    on_progress: Option<&ProgressFn>,
) -> AppResult<ExtractResult> {
    let fmt = ArchiveFormat::from_ext(archive)
        .ok_or_else(|| AppError::UnsupportedFormat(format!("无法识别: {archive}")))?;
    let file = fs::File::open(archive)?;
    let reader = tar_decoder(fmt, file)?;
    let mut ar = Archive::new(reader);
    let wanted: Option<std::collections::HashSet<String>> =
        entries.map(|v| v.iter().cloned().collect());
    fs::create_dir_all(dest)?;
    let mut count = 0usize;

    let total: u64 = list_tar(archive)?
        .iter()
        .filter(|e| !e.is_dir)
        .filter(|e| wanted.as_ref().map(|s| s.contains(&e.name)).unwrap_or(true))
        .map(|e| e.size)
        .sum();

    let mut current = 0u64;
    for entry in ar.entries()? {
        let mut entry = entry.map_err(|e| AppError::Archive(e.to_string()))?;
        let path = entry
            .path()
            .map_err(|e| AppError::Archive(e.to_string()))?
            .to_string_lossy()
            .replace('\\', "/");
        if let Some(set) = &wanted {
            if !set.contains(&path) {
                continue;
            }
        }
        if entry.header().entry_type().is_dir() {
            fs::create_dir_all(Path::new(dest).join(crate::archive::zip::sanitize(&path)))?;
            continue;
        }
        let out = Path::new(dest).join(crate::archive::zip::sanitize(&path));
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut outf = fs::File::create(&out)?;
        std::io::copy(&mut entry, &mut outf)?;
        count += 1;
        current += entry.header().size().unwrap_or(0);
        if let Some(cb) = on_progress {
            cb(current, total);
        }
    }
    Ok(ExtractResult {
        extracted: count,
        dest: dest.to_string(),
    })
}

pub fn list_tar(archive: &str) -> AppResult<Vec<EntryInfo>> {
    let fmt = ArchiveFormat::from_ext(archive)
        .ok_or_else(|| AppError::UnsupportedFormat(format!("无法识别: {archive}")))?;
    let file = fs::File::open(archive)?;
    let reader = tar_decoder(fmt, file)?;
    let mut ar = Archive::new(reader);
    let mut out = Vec::new();
    for entry in ar.entries()? {
        let entry = entry.map_err(|e| AppError::Archive(e.to_string()))?;
        let name = entry
            .path()
            .map_err(|e| AppError::Archive(e.to_string()))?
            .to_string_lossy()
            .replace('\\', "/");
        let is_dir = entry.header().entry_type().is_dir();
        out.push(EntryInfo {
            name,
            size: entry.header().size().unwrap_or(0),
            compressed_size: None,
            is_dir,
            modified: Some(entry.header().mtime().unwrap_or(0)),
            method: Some(fmt.as_str().to_string()),
        });
    }
    Ok(out)
}

pub fn preview_tar(
    archive: &str,
    entry: &str,
    max_bytes: usize,
) -> AppResult<PreviewData> {
    let fmt = ArchiveFormat::from_ext(archive)
        .ok_or_else(|| AppError::UnsupportedFormat(format!("无法识别: {archive}")))?;
    let file = fs::File::open(archive)?;
    let reader = tar_decoder(fmt, file)?;
    let mut ar = Archive::new(reader);
    for e in ar.entries()? {
        let mut e = e.map_err(|e| AppError::Archive(e.to_string()))?;
        let name = e
            .path()
            .map_err(|e| AppError::Archive(e.to_string()))?
            .to_string_lossy()
            .replace('\\', "/");
        if name == entry {
            return read_preview(&mut e, entry, max_bytes);
        }
    }
    Err(AppError::Archive(format!("未找到条目: {entry}")))
}

pub fn test_tar(archive: &str) -> AppResult<TestResult> {
    let fmt = ArchiveFormat::from_ext(archive)
        .ok_or_else(|| AppError::UnsupportedFormat(format!("无法识别: {archive}")))?;
    let file = fs::File::open(archive)?;
    let reader = tar_decoder(fmt, file)?;
    let mut ar = Archive::new(reader);
    let mut entries_out = Vec::new();
    let mut all_ok = true;
    for e in ar.entries()? {
        let mut e = match e {
            Ok(v) => v,
            Err(err) => {
                all_ok = false;
                entries_out.push(EntryTestStatus {
                    name: "<无法读取条目>".into(),
                    ok: false,
                    error: Some(err.to_string()),
                });
                continue;
            }
        };
        let name = e
            .path()
            .map_err(|e| AppError::Archive(e.to_string()))?
            .to_string_lossy()
            .replace('\\', "/");
        match std::io::copy(&mut e, &mut std::io::sink()) {
            Ok(_) => entries_out.push(EntryTestStatus {
                name,
                ok: true,
                error: None,
            }),
            Err(err) => {
                all_ok = false;
                entries_out.push(EntryTestStatus {
                    name,
                    ok: false,
                    error: Some(err.to_string()),
                });
            }
        }
    }
    Ok(TestResult {
        ok: all_ok,
        entries: entries_out,
    })
}

fn io_to_app(e: std::io::Error) -> AppError {
    AppError::Io(e)
}

fn summarize_tar(dest: &str, fmt: ArchiveFormat) -> AppResult<ArchiveInfo> {
    let meta = fs::metadata(dest)?;
    let count = list_tar(dest)?.len();
    Ok(ArchiveInfo {
        path: dest.to_string(),
        format: fmt.as_str().to_string(),
        entry_count: count,
        total_size: meta.len(),
    })
}
