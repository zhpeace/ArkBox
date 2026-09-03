//! RAR 只读支持（列表 / 解压 / 预览 / 完整性检测）。
//!
//! RAR 是专有格式：解码器开源（UnRAR 许可，允许免费解包），编码器从未授权，
//! 因此这里只实现读取侧，创建 RAR 会在分发层直接拒绝。
//!
//! 本模块通过 `unrar` / `unrar_sys` 静态链接官方 UnRAR 源码（版权归
//! Alexander L. Roshal）。按 UnRAR 许可证第 2 条要求，照录以下段落：
//!
//! > UnRAR source code may be used in any software to handle RAR archives
//! > without limitations free of charge, but cannot be used to develop RAR
//! > (WinRAR) compatible archiver and to re-create RAR compression algorithm,
//! > which is proprietary. Distribution of modified UnRAR source code in
//! > separate form or as a part of other software is permitted, provided that
//! > full text of this paragraph, starting from "UnRAR source code" words, is
//! > included in license, or in documentation if license is not available, and
//! > in source code comments of resulting package.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use unrar::error::{Code, UnrarError};
use unrar::{Archive, CursorBeforeHeader, OpenArchive, Process};

use super::zip::{read_preview, sanitize};
use crate::error::{AppError, AppResult};
use crate::model::{
    EntryInfo, EntryTestStatus, ExtractResult, PreviewData, ProgressFn, TestResult,
};

/// 预览时允许的最大单文件体积（RAR 只能整块读出，避免大文件撑爆内存）。
const MAX_PREVIEW_SIZE: u64 = 64 * 1024 * 1024;

/// 打开用于「列举」的压缩包。
fn open_list(
    path: &Path,
    password: Option<&str>,
) -> Result<OpenArchive<unrar::List, CursorBeforeHeader>, UnrarError> {
    match password {
        Some(pw) if !pw.is_empty() => Archive::with_password(path, pw).open_for_listing(),
        _ => Archive::new(path).open_for_listing(),
    }
}

/// 打开用于「读取内容」的压缩包。
fn open_process(
    path: &Path,
    password: Option<&str>,
) -> Result<OpenArchive<Process, CursorBeforeHeader>, UnrarError> {
    match password {
        Some(pw) if !pw.is_empty() => Archive::with_password(path, pw).open_for_processing(),
        _ => Archive::new(path).open_for_processing(),
    }
}

/// unrar 错误 → 应用错误。密码相关单独归类，前端好提示。
///
/// RAR 只加密内容时，密码错与数据损坏都表现为 CRC 错误（UnRAR 不区分），
/// 因此只要调用方传了密码，就把 CRC 错误归到密码问题上，避免误报"文件损坏"。
fn map_err(e: UnrarError, password: Option<&str>) -> AppError {
    let has_pw = matches!(password, Some(p) if !p.is_empty());
    match e.code {
        Code::MissingPassword => AppError::BadPassword("该 RAR 已加密，请提供密码".into()),
        Code::BadPassword => AppError::BadPassword("密码错误，无法解压 RAR".into()),
        Code::BadData | Code::BadArchive if has_pw => {
            AppError::BadPassword(format!("密码错误或文件已损坏（RAR 两者报同样的 CRC 错误）: {e}"))
        }
        Code::BadData | Code::BadArchive => AppError::Archive(format!("RAR 文件损坏: {e}")),
        Code::UnknownFormat => AppError::Archive("不是有效的 RAR 文件（可能是分卷中间卷）".into()),
        Code::EOpen => AppError::Archive(format!("无法打开 RAR: {e}")),
        _ => AppError::Archive(format!("RAR 处理失败: {e}")),
    }
}

/// 分卷包必须从第一卷开始读，这里把路径归一化到第一卷。
/// 第一卷缺失时直接给出可读的错误，而不是让 UnRAR 抛 "Could not open archive"。
fn resolve_first_part(archive: &str) -> AppResult<PathBuf> {
    let p = PathBuf::from(archive);
    let a = Archive::new(&p);
    if a.is_multipart() {
        let first = a.first_part();
        if !first.exists() {
            return Err(AppError::Archive(format!(
                "分卷不完整：找不到第一卷 {}，请把整个分卷系列放在一起后打开第一卷",
                first.display()
            )));
        }
        return Ok(first);
    }
    Ok(p)
}

/// RAR 内部用 `\` 分隔路径，Unix 侧统一成 `/`。
fn normalize<P: AsRef<Path>>(name: P) -> String {
    name.as_ref().to_string_lossy().replace('\\', "/")
}

/// MS-DOS 时间戳（RAR 头部格式）→ Unix 秒。
fn dos_to_unix(t: u32) -> Option<u64> {
    if t == 0 {
        return None;
    }
    let year = ((t >> 25) & 0x7f) as i32 + 1980;
    let month = ((t >> 21) & 0x0f) as i32;
    let day = ((t >> 16) & 0x1f) as i32;
    let hour = ((t >> 11) & 0x1f) as i32;
    let minute = ((t >> 5) & 0x3f) as i32;
    let second = ((t & 0x1f) as i32) * 2;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let secs = days * 86400 + hour as i64 * 3600 + minute as i64 * 60 + second as i64;
    if secs < 0 {
        None
    } else {
        Some(secs as u64)
    }
}

/// Howard Hinnant 的 civil date → days since epoch 算法。
fn days_from_civil(y: i32, m: i32, d: i32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) as i64 / 400;
    let yoe = y as i64 - era * 400; // [0, 399]
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) as i64 / 5 + d as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// 压缩方法编号（RAR4 是 0x30 起，RAR5 是 0 起）。
fn method_name(m: u32) -> String {
    let n = if m >= 0x30 { m - 0x30 } else { m };
    match n {
        0 => "存储".to_string(),
        1..=5 => format!("RAR m{n}"),
        _ => format!("未知({m})"),
    }
}

/// 列出 RAR 内所有条目（不解压内容）。
pub fn list_rar(archive: &str, password: Option<&str>) -> AppResult<Vec<EntryInfo>> {
    let path = resolve_first_part(archive)?;
    let mut out: Vec<EntryInfo> = Vec::new();
    for item in open_list(&path, password).map_err(|e| map_err(e, password))? {
        let h = item.map_err(|e| map_err(e, password))?;
        out.push(EntryInfo {
            name: normalize(&h.filename),
            size: h.unpacked_size,
            // UnRAR 不暴露压缩后大小
            compressed_size: None,
            is_dir: h.is_directory(),
            modified: dos_to_unix(h.file_time),
            method: Some(method_name(h.method)),
        });
    }
    Ok(out)
}

/// 解压 RAR。entries 为 None 时解压全部。
pub fn extract_rar(
    archive: &str,
    dest: &str,
    entries: Option<&[String]>,
    password: Option<&str>,
    on_progress: Option<&ProgressFn>,
) -> AppResult<ExtractResult> {
    let path = resolve_first_part(archive)?;
    let wanted: Option<HashSet<String>> = entries.map(|v| v.iter().cloned().collect());
    let dest = Path::new(dest);
    fs::create_dir_all(dest)?;

    // 先列一遍拿到分母（list 模式只跳内容，代价小）
    let total: u64 = list_rar(archive, password)?
        .iter()
        .filter(|e| !e.is_dir)
        .filter(|e| match &wanted {
            Some(set) => set.contains(&e.name),
            None => true,
        })
        .map(|e| e.size)
        .sum();

    let mut current = 0u64;
    let mut count = 0usize;
    let mut cursor = open_process(&path, password).map_err(|e| map_err(e, password))?;

    loop {
        let header_view = match cursor.read_header().map_err(|e| map_err(e, password))? {
            Some(h) => h,
            None => break,
        };
        let name = normalize(&header_view.entry().filename);
        let is_dir = header_view.entry().is_directory();
        let size = header_view.entry().unpacked_size;

        let wanted_this = match &wanted {
            Some(set) => set.contains(&name),
            None => true,
        };

        // 路径含 NUL 会让底层 CString 构造 panic（release 用 panic=abort），必须提前拦掉
        if !wanted_this || name.contains('\0') {
            cursor = header_view.skip().map_err(|e| map_err(e, password))?;
            continue;
        }

        let out_path = dest.join(sanitize(&name));
        if is_dir {
            fs::create_dir_all(&out_path)?;
            cursor = header_view.skip().map_err(|e| map_err(e, password))?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)?;
        }
        cursor = header_view.extract_to(&out_path).map_err(|e| map_err(e, password))?;
        count += 1;
        current += size;
        if let Some(cb) = on_progress {
            cb(current, total);
        }
    }

    Ok(ExtractResult {
        extracted: count,
        dest: dest.to_string_lossy().to_string(),
    })
}

/// 不解压预览 RAR 内某个条目。
pub fn preview_rar(
    archive: &str,
    entry: &str,
    password: Option<&str>,
    max_bytes: usize,
) -> AppResult<PreviewData> {
    let path = resolve_first_part(archive)?;
    let mut cursor = open_process(&path, password).map_err(|e| map_err(e, password))?;

    loop {
        let header_view = match cursor.read_header().map_err(|e| map_err(e, password))? {
            Some(h) => h,
            None => break,
        };
        let name = normalize(&header_view.entry().filename);
        if name != entry {
            cursor = header_view.skip().map_err(|e| map_err(e, password))?;
            continue;
        }
        if header_view.entry().is_directory() {
            return Err(AppError::Archive("该条目是目录，无法预览".into()));
        }
        if header_view.entry().unpacked_size > MAX_PREVIEW_SIZE {
            return Err(AppError::Archive(format!(
                "该条目超过 {} MB，请先解压后再查看",
                MAX_PREVIEW_SIZE / 1024 / 1024
            )));
        }
        let (data, _next) = header_view.read().map_err(|e| map_err(e, password))?;
        let mut slice = data.as_slice();
        return read_preview(&mut slice, entry, max_bytes);
    }

    Err(AppError::Archive(format!("未找到条目: {entry}")))
}

/// 逐条目校验 CRC（不落盘）。
pub fn test_rar(archive: &str, password: Option<&str>) -> AppResult<TestResult> {
    let path = resolve_first_part(archive)?;
    let mut cursor = open_process(&path, password).map_err(|e| map_err(e, password))?;
    let mut out: Vec<EntryTestStatus> = Vec::new();
    let mut all_ok = true;

    loop {
        let header_view = match cursor.read_header().map_err(|e| map_err(e, password))? {
            Some(h) => h,
            None => break,
        };
        let name = normalize(&header_view.entry().filename);
        if header_view.entry().is_directory() {
            cursor = match header_view.skip() {
                Ok(c) => c,
                Err(e) => {
                    all_ok = false;
                    out.push(EntryTestStatus {
                        name,
                        ok: false,
                        error: Some(map_err(e, password).to_string()),
                    });
                    break;
                }
            };
            continue;
        }
        match header_view.test() {
            Ok(c) => {
                cursor = c;
                out.push(EntryTestStatus {
                    name,
                    ok: true,
                    error: None,
                });
            }
            Err(e) => {
                // 校验失败后游标状态不可信，停止后续检测
                all_ok = false;
                out.push(EntryTestStatus {
                    name,
                    ok: false,
                    error: Some(map_err(e, password).to_string()),
                });
                break;
            }
        }
    }

    Ok(TestResult {
        ok: all_ok,
        entries: out,
    })
}

#[cfg(test)]
mod tests {
    use super::{days_from_civil, dos_to_unix, method_name, normalize};
    use std::path::PathBuf;

    #[test]
    fn civil_epoch_anchor() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(1970, 1, 2), 1);
        assert_eq!(days_from_civil(2000, 3, 1), 11017);
    }

    #[test]
    fn dos_time_roundtrip() {
        // 2024-01-02 03:04:06 UTC → DOS: year=44, month=1, day=2, h=3, m=4, s/2=3
        // 期望值由 `date -u -j -f "%Y-%m-%d %H:%M:%S" "2024-01-02 03:04:06" "+%s"` 得到
        let dos = (44u32 << 25) | (1 << 21) | (2 << 16) | (3 << 11) | (4 << 5) | 3;
        assert_eq!(dos_to_unix(dos), Some(1_704_164_646));
        assert_eq!(dos_to_unix(0), None);
        // 月份非法应返回 None，不能算出离谱时间
        assert_eq!(dos_to_unix((44u32 << 25) | (13 << 21) | (2 << 16)), None);
    }

    #[test]
    fn path_normalize() {
        assert_eq!(normalize(&PathBuf::from("a\\b\\c.txt")), "a/b/c.txt");
        assert_eq!(normalize(&PathBuf::from("a/b.txt")), "a/b.txt");
    }

    #[test]
    fn method_label() {
        assert_eq!(method_name(0), "存储");
        assert_eq!(method_name(0x33), "RAR m3");
        assert_eq!(method_name(3), "RAR m3");
    }
}
