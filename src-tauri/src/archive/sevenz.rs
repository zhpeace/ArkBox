use std::fs;
use std::io::{copy, Read};
use std::path::{Path, PathBuf};

use base64::Engine;
use sevenz_rust::{
    AesEncoderOptions, Password, SevenZArchiveEntry, SevenZMethod, SevenZMethodConfiguration,
    SevenZReader, SevenZWriter,
};

use crate::archive::should_exclude;
use crate::archive::zip::{guess_mime, looks_binary, sanitize};
use crate::error::{AppError, AppResult};
use crate::model::{
    ArchiveInfo, EntryInfo, EntryTestStatus, ExtractResult, PreviewData, ProgressFn, TestResult,
};

pub fn compress_7z(
    paths: &[String],
    dest: &str,
    password: Option<&str>,
    _level: u8,
    exclude: &[String],
    compress_hidden: bool,
    on_progress: Option<&ProgressFn>,
) -> AppResult<ArchiveInfo> {
    if paths.is_empty() {
        return Err(AppError::Archive("未提供任何待压缩路径".into()));
    }
    let file = fs::File::create(dest)
        .map_err(|e| AppError::Io(std::io::Error::new(e.kind(), format!("{dest}: {e}"))))?;
    let mut sz = SevenZWriter::new(file)
        .map_err(|e| AppError::Archive(format!("创建 7z 失败: {e}")))?;

    if has_password(password) {
        // 方法链顺序即数据流方向：原始内容 -> LZMA2 压缩 -> AES 加密 -> 落盘。
        // 注意 writer 还会用同一份 AES 配置去加密文件头（7z 的标准做法），
        // 所以加密后不带密码连文件名都列不出来，这也意味着 summarize 必须带密码。
        let mut methods = Vec::with_capacity(2);
        methods.push(aes_method(password.unwrap_or("")));
        methods.push(SevenZMethodConfiguration::new(SevenZMethod::LZMA2));
        sz.set_content_methods(methods);
    }

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
        add_7z(&mut sz, &abs, &base, exclude, compress_hidden, &mut done, total, on_progress)?;
    }
    sz.finish()
        .map_err(|e| AppError::Archive(format!("写入 7z 失败: {e}")))?;
    summarize_7z(dest, password)
}

/// 7z 的 AES-256 加密配置。
///
/// `num_cycles_power` 是密钥派生的迭代次数幂（7-Zip 官方为 19，即 2^19 次
/// SHA-256 迭代）；sevenz-rust 默认是 8，密钥派生太弱，这里对齐官方值。
/// 代价是每次派生约 100ms（release）：列目录 1 次、解压每个文件再各 1 次。
///
/// ⚠️ 上游限制：sevenz-rust 解密文件头时不校验结果，实测**任意非空密码都能
/// 列出文件名与大小**，只有真正取内容时才会因密码错误而失败。即 7z 加密保护
/// 数据、不保护元数据（7-Zip 官方是连文件名一起保护的）。属 crate 缺陷，
/// 无法从调用侧绕过，只能在使用说明里讲清楚。
fn aes_method(password: &str) -> SevenZMethodConfiguration {
    let mut opts = AesEncoderOptions::new(Password::from(password));
    opts.num_cycles_power = 19;
    opts.into()
}

fn add_7z(
    sz: &mut SevenZWriter<fs::File>,
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
        let entry = SevenZArchiveEntry::from_path(abs, rel_str.clone());
        sz.push_archive_entry::<std::io::Empty>(entry, None)
            .map_err(|e| AppError::Archive(e.to_string()))?;
        for child in fs::read_dir(abs)? {
            let child = child?;
            add_7z(sz, &child.path(), base, exclude, compress_hidden, done, total, on_progress)?;
        }
    } else {
        let entry = SevenZArchiveEntry::from_path(abs, rel_str);
        let content = fs::File::open(abs)?;
        sz.push_archive_entry(entry, Some(content))
            .map_err(|e| AppError::Archive(e.to_string()))?;
        *done += 1;
        if let Some(cb) = on_progress {
            cb(*done as u64, total as u64);
        }
    }
    Ok(())
}

pub fn extract_7z(
    archive: &str,
    dest: &str,
    entries: Option<&[String]>,
    password: Option<&str>,
    on_progress: Option<&ProgressFn>,
) -> AppResult<ExtractResult> {
    let mut reader = SevenZReader::open(archive, password_password(password))
        .map_err(|e| map_7z_err(e, "无法打开 7z"))?;
    let wanted: Option<std::collections::HashSet<String>> =
        entries.map(|v| v.iter().cloned().collect());
    fs::create_dir_all(dest)?;
    let mut count = 0usize;

    let total: u64 = list_7z(archive, password)?
        .iter()
        .filter(|e| !e.is_dir)
        .filter(|e| wanted.as_ref().map(|s| s.contains(&e.name)).unwrap_or(true))
        .map(|e| e.size)
        .sum();

    let mut current = 0u64;
    reader
        .for_each_entries(|entry, read| {
            let name = entry.name().to_string();
            if entry.is_directory() {
                fs::create_dir_all(Path::new(dest).join(sanitize(&name)))?;
                return Ok(true);
            }
            if let Some(set) = &wanted {
                if !set.contains(&name) {
                    return Ok(true);
                }
            }
            let out = Path::new(dest).join(sanitize(&name));
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut outf = fs::File::create(&out)?;
            copy(read, &mut outf)?;
            count += 1;
            current += entry.size();
            if let Some(cb) = on_progress {
                cb(current, total);
            }
            Ok(true)
        })
        .map_err(|e| map_7z_err(e, "解压失败"))?;

    Ok(ExtractResult {
        extracted: count,
        dest: dest.to_string(),
    })
}

pub fn list_7z(archive: &str, password: Option<&str>) -> AppResult<Vec<EntryInfo>> {
    let mut reader = SevenZReader::open(archive, password_password(password))
        .map_err(|e| map_7z_err(e, "无法打开 7z"))?;
    let mut out = Vec::new();
    reader
        .for_each_entries(|entry, _read| {
            out.push(EntryInfo {
                name: entry.name().to_string(),
                size: entry.size(),
                compressed_size: None,
                is_dir: entry.is_directory(),
                modified: None,
                method: Some("7z".into()),
            });
            Ok(true)
        })
        .map_err(|e| map_7z_err(e, "读取 7z 失败"))?;
    Ok(out)
}

pub fn preview_7z(
    archive: &str,
    entry: &str,
    password: Option<&str>,
    max_bytes: usize,
) -> AppResult<PreviewData> {
    let mut reader = SevenZReader::open(archive, password_password(password))
        .map_err(|e| map_7z_err(e, "无法打开 7z"))?;
    let mut found: Option<PreviewData> = None;
    reader
        .for_each_entries(|e, read| {
            if e.name() != entry {
                return Ok(true);
            }
            if e.is_directory() {
                return Err(sevenz_rust::Error::Other("该条目是目录，无法预览".into()));
            }
            let mut buf = Vec::new();
            read.take(max_bytes as u64).read_to_end(&mut buf)?;
            let is_binary = looks_binary(&buf);
            let mime = guess_mime(entry, is_binary);
            let data = if is_binary {
                PreviewData {
                    name: entry.to_string(),
                    mime,
                    text: None,
                    data_base64: Some(base64::engine::general_purpose::STANDARD.encode(&buf)),
                    truncated: buf.len() >= max_bytes,
                    is_binary: true,
                }
            } else {
                let truncated = buf.len() >= max_bytes;
                PreviewData {
                    name: entry.to_string(),
                    mime,
                    text: Some(String::from_utf8_lossy(&buf).to_string()),
                    data_base64: None,
                    truncated,
                    is_binary: false,
                }
            };
            found = Some(data);
            Ok(false)
        })
        .map_err(|e| AppError::Archive(e.to_string()))?;
    found.ok_or_else(|| AppError::Archive(format!("未找到条目: {entry}")))
}

pub fn test_7z(archive: &str, password: Option<&str>) -> AppResult<TestResult> {
    let mut reader = SevenZReader::open(archive, password_password(password))
        .map_err(|e| map_7z_err(e, "无法打开 7z"))?;
    let mut entries_out = Vec::new();
    let mut all_ok = true;
    reader
        .for_each_entries(|entry, read| {
            let name = entry.name().to_string();
            let mut sink = Vec::new();
            match copy(read, &mut sink) {
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
            Ok(true)
        })
        .map_err(|e| map_7z_err(e, "检测失败"))?;
    Ok(TestResult {
        ok: all_ok,
        entries: entries_out,
    })
}

fn password_password(password: Option<&str>) -> Password {
    Password::from(password.unwrap_or(""))
}

/// 7z 错误 → 应用错误。
///
/// 加密包的密码问题单独归类：`PasswordRequired` 表示没给密码，
/// `MaybeBadPassword` 表示解出来的数据不通（密码错或文件坏，7z 无法区分）。
fn map_7z_err(e: sevenz_rust::Error, ctx: &str) -> AppError {
    match e {
        sevenz_rust::Error::PasswordRequired => {
            AppError::BadPassword("该 7z 已加密，请提供密码".into())
        }
        sevenz_rust::Error::MaybeBadPassword(io) => {
            AppError::BadPassword(format!("密码错误或文件已损坏: {io}"))
        }
        other => AppError::Archive(format!("{ctx}: {other}")),
    }
}

/// 密码是否有实际内容（空串视为没填）。
fn has_password(password: Option<&str>) -> bool {
    matches!(password, Some(p) if !p.is_empty())
}

fn summarize_7z(dest: &str, password: Option<&str>) -> AppResult<ArchiveInfo> {
    let meta = fs::metadata(dest)?;
    let count = list_7z(dest, password)?.len();
    Ok(ArchiveInfo {
        path: dest.to_string(),
        format: "7z".into(),
        entry_count: count,
        total_size: meta.len(),
    })
}
