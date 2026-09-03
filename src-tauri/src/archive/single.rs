//! 独立单文件流式压缩：.gz / .bz2 / .xz / .zst
//! 与 tar.* 组合不同，这里不打包、只把「单个文件」压缩成一条流。

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::error::{AppError, AppResult};
use crate::model::{ArchiveFormat, ArchiveInfo, EntryInfo, EntryTestStatus, ExtractResult, PreviewData, ProgressFn, TestResult};

/// 解压后的文件名：去掉压缩扩展名（与 gunzip 行为一致）。
fn derived_name(archive: &str) -> String {
    let p = Path::new(archive);
    let stem = p
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    let lower = stem.to_ascii_lowercase();
    for suf in [".gz", ".bz2", ".xz", ".zst"] {
        if lower.ends_with(suf) {
            return stem[..stem.len() - suf.len()].to_string();
        }
    }
    stem
}

fn open_decoder(fmt: ArchiveFormat, src: File) -> AppResult<Box<dyn Read>> {
    Ok(match fmt {
        ArchiveFormat::Gz => Box::new(flate2::read::GzDecoder::new(src)),
        ArchiveFormat::Bz2 => Box::new(bzip2::read::BzDecoder::new(src)),
        ArchiveFormat::Xz => Box::new(xz2::read::XzDecoder::new(src)),
        ArchiveFormat::Zstd => Box::new(zstd::stream::read::Decoder::new(src).map_err(AppError::Io)?),
        _ => return Err(AppError::UnsupportedFormat("非单文件格式".into())),
    })
}

/// 压缩单个文件为 .gz/.bz2/.xz/.zst。
pub fn compress_single(
    paths: &[String],
    dest: &str,
    fmt: ArchiveFormat,
    level: u8,
    on_progress: Option<&ProgressFn>,
) -> AppResult<ArchiveInfo> {
    if paths.len() != 1 {
        return Err(AppError::Archive(
            "单文件格式（gz/bz2/xz/zst）仅支持压缩单个文件，多文件请用 tar.*".into(),
        ));
    }
    let src_path = &paths[0];
    let meta = fs::metadata(src_path).map_err(AppError::Io)?;
    if meta.is_dir() {
        return Err(AppError::Archive(
            "单文件格式不支持目录，请改用 tar.gz / tar.bz2 等组合格式".into(),
        ));
    }
    let src = File::open(src_path).map_err(AppError::Io)?;
    let dest_file = File::create(dest).map_err(AppError::Io)?;
    let total_size = meta.len();

    if let Some(cb) = on_progress {
        cb(0, 1);
    }
    match fmt {
        ArchiveFormat::Gz => {
            let mut enc =
                flate2::write::GzEncoder::new(dest_file, flate2::Compression::new(level as u32));
            io::copy(&mut src.take(u64::MAX), &mut enc).map_err(AppError::Io)?;
            enc.try_finish().map_err(AppError::Io)?;
        }
        ArchiveFormat::Bz2 => {
            let mut enc = bzip2::write::BzEncoder::new(dest_file, bzip2::Compression::new(level as u32));
            io::copy(&mut src.take(u64::MAX), &mut enc).map_err(AppError::Io)?;
            enc.try_finish().map_err(AppError::Io)?;
        }
        ArchiveFormat::Xz => {
            let mut enc = xz2::write::XzEncoder::new(dest_file, level as u32);
            io::copy(&mut src.take(u64::MAX), &mut enc).map_err(AppError::Io)?;
            enc.try_finish().map_err(AppError::Io)?;
        }
        ArchiveFormat::Zstd => {
            let mut enc = zstd::stream::write::Encoder::new(dest_file, level as i32)
                .map_err(AppError::Io)?;
            io::copy(&mut src.take(u64::MAX), &mut enc).map_err(AppError::Io)?;
            enc.finish().map_err(AppError::Io)?;
        }
        _ => return Err(AppError::UnsupportedFormat("非单文件格式".into())),
    }
    if let Some(cb) = on_progress {
        cb(1, 1);
    }

    Ok(ArchiveInfo {
        path: dest.to_string(),
        format: fmt.as_str().to_string(),
        entry_count: 1,
        total_size,
    })
}

/// 解压单文件流到目标目录（文件名取派生名）。
pub fn extract_single(
    archive: &str,
    dest: &str,
    fmt: ArchiveFormat,
    on_progress: Option<&ProgressFn>,
) -> AppResult<ExtractResult> {
    let src = File::open(archive).map_err(AppError::Io)?;
    let total = fs::metadata(archive).map_err(AppError::Io)?.len();
    let mut dec = open_decoder(fmt, src)?;
    let out_path: PathBuf = Path::new(dest).join(derived_name(archive));
    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent).map_err(AppError::Io)?;
    }
    let mut out = File::create(&out_path).map_err(AppError::Io)?;
    let mut buf = [0u8; 64 * 1024];
    let mut current = 0u64;
    loop {
        let n = dec.read(&mut buf).map_err(AppError::Io)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(AppError::Io)?;
        current += n as u64;
        if let Some(cb) = on_progress {
            cb(current, total);
        }
    }
    Ok(ExtractResult {
        extracted: 1,
        dest: dest.to_string(),
    })
}

pub fn list_single(archive: &str, fmt: ArchiveFormat) -> AppResult<Vec<EntryInfo>> {
    let meta = fs::metadata(archive).map_err(AppError::Io)?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    let method = match fmt {
        ArchiveFormat::Gz => "gzip",
        ArchiveFormat::Bz2 => "bzip2",
        ArchiveFormat::Xz => "xz",
        ArchiveFormat::Zstd => "zstd",
        _ => "?",
    };
    Ok(vec![EntryInfo {
        name: derived_name(archive),
        size: 0,
        compressed_size: Some(meta.len()),
        is_dir: false,
        modified: mtime,
        method: Some(method.into()),
    }])
}

pub fn preview_single(
    archive: &str,
    fmt: ArchiveFormat,
    max_bytes: usize,
) -> AppResult<PreviewData> {
    let src = File::open(archive).map_err(AppError::Io)?;
    let mut dec = open_decoder(fmt, src)?;
    let mut buf = Vec::new();
    let mut limited = dec.take(max_bytes as u64 + 1);
    limited.read_to_end(&mut buf).map_err(AppError::Io)?;
    let truncated = buf.len() > max_bytes;
    if truncated {
        buf.truncate(max_bytes);
    }
    let name = derived_name(archive);

    let is_binary = match String::from_utf8(buf.clone()) {
        Ok(s) => s.contains('\0'),
        Err(_) => true,
    };

    if is_binary {
        let mime = mime_for(&name);
        Ok(PreviewData {
            name,
            mime,
            text: None,
            data_base64: Some(base64::encode(&buf)),
            truncated,
            is_binary: true,
        })
    } else {
        Ok(PreviewData {
            name,
            mime: "text/plain".into(),
            text: Some(String::from_utf8_lossy(&buf).to_string()),
            data_base64: None,
            truncated,
            is_binary: false,
        })
    }
}

pub fn test_single(archive: &str, fmt: ArchiveFormat) -> AppResult<TestResult> {
    let name = derived_name(archive);
    let src = File::open(archive).map_err(AppError::Io)?;
    let mut dec = open_decoder(fmt, src)?;
    match io::copy(&mut dec, &mut io::sink()) {
        Ok(_) => Ok(TestResult {
            ok: true,
            entries: vec![EntryTestStatus {
                name,
                ok: true,
                error: None,
            }],
        }),
        Err(e) => Ok(TestResult {
            ok: false,
            entries: vec![EntryTestStatus {
                name,
                ok: false,
                error: Some(e.to_string()),
            }],
        }),
    }
}

fn mime_for(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".png") {
        "image/png".into()
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg".into()
    } else if lower.ends_with(".gif") {
        "image/gif".into()
    } else if lower.ends_with(".pdf") {
        "application/pdf".into()
    } else {
        "application/octet-stream".into()
    }
}
