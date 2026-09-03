use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// 进度回调：当前已处理字节数 / 总字节数（total=0 表示不确定进度，前端用脉冲动画）。
pub type ProgressFn = Arc<dyn Fn(u64, u64) + Send + Sync>;

/// 推给前端的进度事件载荷。
#[derive(Serialize, Clone, Debug)]
pub struct ProgressPayload {
    /// "compress" | "extract"
    pub kind: String,
    pub current: u64,
    pub total: u64,
}

/// 支持的压缩格式。命令层用字符串收发，内部转成枚举。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    SevenZip,
    TarGz,
    TarBz,
    TarXz,
    TarZstd,
    Gz,
    Bz2,
    Xz,
    Zstd,
    /// RAR 只能读不能写（编码器为专有实现，从未开源授权）。
    Rar,
}

/// 判断文件名是否属于 RAR 家族：`.rar`、`.part1.rar`、旧式 `.r01` / `.001` 分卷。
/// 数字后缀限定 3 位，避免 `report.2024` 之类的普通文件被误判。
fn is_rar_name(p: &str) -> bool {
    if p.ends_with(".rar") {
        return true;
    }
    let ext = match p.rsplit('.').next() {
        Some(e) => e,
        None => return false,
    };
    if ext.len() == 3 && ext.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    ext.len() == 3
        && (ext.starts_with('r') || ext.starts_with('R'))
        && ext[1..].chars().all(|c| c.is_ascii_digit())
}

impl ArchiveFormat {
    pub fn from_ext(path: &str) -> Option<ArchiveFormat> {
        let p = path.to_ascii_lowercase();
        if p.ends_with(".zip") || p.ends_with(".zipx") {
            Some(ArchiveFormat::Zip)
        } else if p.ends_with(".7z") {
            Some(ArchiveFormat::SevenZip)
        } else if p.ends_with(".tar.gz") || p.ends_with(".tgz") {
            Some(ArchiveFormat::TarGz)
        } else if p.ends_with(".tar.bz2") || p.ends_with(".tbz") || p.ends_with(".tbz2") {
            Some(ArchiveFormat::TarBz)
        } else if p.ends_with(".tar.xz") || p.ends_with(".txz") {
            Some(ArchiveFormat::TarXz)
        } else if p.ends_with(".tar.zst") || p.ends_with(".tzst") {
            Some(ArchiveFormat::TarZstd)
        } else if p.ends_with(".gz") {
            Some(ArchiveFormat::Gz)
        } else if p.ends_with(".bz2") {
            Some(ArchiveFormat::Bz2)
        } else if p.ends_with(".xz") {
            Some(ArchiveFormat::Xz)
        } else if p.ends_with(".zst") {
            Some(ArchiveFormat::Zstd)
        } else if is_rar_name(&p) {
            Some(ArchiveFormat::Rar)
        } else {
            None
        }
    }

    pub fn from_str(s: &str) -> Option<ArchiveFormat> {
        match s.to_ascii_lowercase().as_str() {
            "zip" | "zipx" => Some(ArchiveFormat::Zip),
            "7z" | "sevenzip" => Some(ArchiveFormat::SevenZip),
            "targz" | "tgz" | "tar.gz" => Some(ArchiveFormat::TarGz),
            "tarbz" | "tbz" | "tar.bz2" => Some(ArchiveFormat::TarBz),
            "tarxz" | "txz" | "tar.xz" => Some(ArchiveFormat::TarXz),
            "tarzstd" | "tzst" | "tar.zst" => Some(ArchiveFormat::TarZstd),
            "gz" => Some(ArchiveFormat::Gz),
            "bz2" => Some(ArchiveFormat::Bz2),
            "xz" => Some(ArchiveFormat::Xz),
            "zst" => Some(ArchiveFormat::Zstd),
            "rar" => Some(ArchiveFormat::Rar),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ArchiveFormat::Zip => "zip",
            ArchiveFormat::SevenZip => "7z",
            ArchiveFormat::TarGz => "tar.gz",
            ArchiveFormat::TarBz => "tar.bz2",
            ArchiveFormat::TarXz => "tar.xz",
            ArchiveFormat::TarZstd => "tar.zst",
            ArchiveFormat::Gz => "gz",
            ArchiveFormat::Bz2 => "bz2",
            ArchiveFormat::Xz => "xz",
            ArchiveFormat::Zstd => "zst",
            ArchiveFormat::Rar => "rar",
        }
    }
}

/// 压缩包内单个条目的元信息（不解压即可获得）。
#[derive(Serialize, Clone, Debug)]
pub struct EntryInfo {
    pub name: String,
    pub size: u64,
    pub compressed_size: Option<u64>,
    pub is_dir: bool,
    pub modified: Option<u64>,
    pub method: Option<String>,
}

/// 压缩完成后的概要信息。
#[derive(Serialize, Clone, Debug)]
pub struct ArchiveInfo {
    pub path: String,
    pub format: String,
    pub entry_count: usize,
    pub total_size: u64,
}

/// 解压结果。
#[derive(Serialize, Clone, Debug)]
pub struct ExtractResult {
    pub extracted: usize,
    pub dest: String,
}

/// 不解压预览某个条目的内容。文本直接给 text；二进制给 data_base64 + mime。
#[derive(Serialize, Clone, Debug)]
pub struct PreviewData {
    pub name: String,
    pub mime: String,
    pub text: Option<String>,
    pub data_base64: Option<String>,
    pub truncated: bool,
    pub is_binary: bool,
}

/// 单条目的完整性检测结果。
#[derive(Serialize, Clone, Debug)]
pub struct EntryTestStatus {
    pub name: String,
    pub ok: bool,
    pub error: Option<String>,
}

/// 整包完整性检测结果。
#[derive(Serialize, Clone, Debug)]
pub struct TestResult {
    pub ok: bool,
    pub entries: Vec<EntryTestStatus>,
}

/// 可复用的压缩预设。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PresetConfig {
    pub name: String,
    pub format: String,
    pub password: Option<String>,
    pub level: u8,
    pub exclude: Vec<String>,
    pub compress_hidden: bool,
}

/// 直接编辑包内文件的句柄。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EditHandle {
    pub handle_id: String,
    pub temp_path: String,
    pub entry_name: String,
}
