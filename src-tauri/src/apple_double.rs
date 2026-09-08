//! AppleDouble 编解码（用于 zip 的 `__MACOSX` 元数据保真）。
//!
//! 格式见 Apple 技术说明：magic `0x00051607` + version `0x00020000` + 16 字节 filler
//! + entry 表（每项 id/offset/len）+ 各 entry 数据。
//!
//! 我们固定写两个 entry：
//! - id 9  Finder Info（32 字节，取自 `com.apple.FinderInfo`）
//! - id 10 ArkBox 自有容器：其余 xattr 的 length-prefixed 二进制序列化
//!
//! 这样即便系统工具不识别 id 10，也能安全忽略而不损坏结构；ArkBox 解压时读取
//! id 10 还原全部扩展属性，实现元数据 round-trip。

use std::collections::BTreeMap;

const MAGIC: u32 = 0x0005_1607;
const VERSION: u32 = 0x0002_0000;
const ENTRY_FINDER_INFO: u32 = 9;
const ENTRY_XATTR_BLOB: u32 = 10;

/// 构造 AppleDouble：把 `finder_info`（com.apple.FinderInfo）放进 entry 9，
/// `others`（其余 xattr）放进 entry 10。
pub fn build(finder_info: &[u8; 32], others: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let entries: Vec<(u32, Vec<u8>)> = vec![
        (ENTRY_FINDER_INFO, finder_info.to_vec()),
        (ENTRY_XATTR_BLOB, encode_xattrs(others)),
    ];
    serialize(&entries)
}

/// 解析 AppleDouble，返回 (finder_info, 其余 xattr)。无法识别时返回 None。
pub fn parse(buf: &[u8]) -> Option<([u8; 32], BTreeMap<String, Vec<u8>>)> {
    let entries = deserialize(buf)?;
    let mut finder_info = [0u8; 32];
    let mut others = BTreeMap::new();
    for (id, data) in entries {
        if id == ENTRY_FINDER_INFO {
            let n = data.len().min(32);
            finder_info[..n].copy_from_slice(&data[..n]);
        } else if id == ENTRY_XATTR_BLOB {
            if let Some(map) = decode_xattrs(&data) {
                others = map;
            }
        }
    }
    Some((finder_info, others))
}

/// 其余 xattr → 二进制：每段 = name(utf8) + 0x00 + u32 BE 长度 + value。
fn encode_xattrs(map: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, value) in map {
        out.extend_from_slice(name.as_bytes());
        out.push(0);
        out.extend_from_slice(&(value.len() as u32).to_be_bytes());
        out.extend_from_slice(value);
    }
    out
}

fn decode_xattrs(buf: &[u8]) -> Option<BTreeMap<String, Vec<u8>>> {
    let mut map = BTreeMap::new();
    let mut i = 0;
    while i < buf.len() {
        let start = i;
        let mut end = i;
        while end < buf.len() && buf[end] != 0 {
            end += 1;
        }
        if end >= buf.len() {
            break;
        }
        let name = String::from_utf8(buf[start..end].to_vec()).ok()?;
        i = end + 1;
        if i + 4 > buf.len() {
            break;
        }
        let len = u32::from_be_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]) as usize;
        i += 4;
        if i + len > buf.len() {
            break;
        }
        let value = buf[i..i + len].to_vec();
        i += len;
        map.insert(name, value);
    }
    Some(map)
}

fn serialize(entries: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let header_len = 26usize;
    let table_len = entries.len() * 12;
    let mut body = Vec::new();
    let mut offsets = Vec::with_capacity(entries.len());
    let mut cursor = header_len + table_len;
    for (_id, data) in entries {
        offsets.push(cursor as u32);
        body.extend_from_slice(data);
        cursor += data.len();
    }
    let mut out = Vec::with_capacity(header_len + table_len + body.len());
    out.extend_from_slice(&MAGIC.to_be_bytes());
    out.extend_from_slice(&VERSION.to_be_bytes());
    out.extend_from_slice(&[0u8; 16]);
    out.extend_from_slice(&(entries.len() as u16).to_be_bytes());
    for (idx, (id, _)) in entries.iter().enumerate() {
        out.extend_from_slice(&id.to_be_bytes());
        out.extend_from_slice(&offsets[idx].to_be_bytes());
        out.extend_from_slice(&(entries[idx].1.len() as u32).to_be_bytes());
    }
    out.extend_from_slice(&body);
    out
}

fn deserialize(buf: &[u8]) -> Option<Vec<(u32, Vec<u8>)>> {
    if buf.len() < 26 {
        return None;
    }
    let magic = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
    if magic != MAGIC {
        return None;
    }
    let num = u16::from_be_bytes([buf[24], buf[25]]) as usize;
    let mut entries = Vec::with_capacity(num);
    let mut p = 26;
    for _ in 0..num {
        if p + 12 > buf.len() {
            return None;
        }
        let id = u32::from_be_bytes([buf[p], buf[p + 1], buf[p + 2], buf[p + 3]]);
        let off = u32::from_be_bytes([buf[p + 4], buf[p + 5], buf[p + 6], buf[p + 7]]) as usize;
        let len = u32::from_be_bytes([buf[p + 8], buf[p + 9], buf[p + 10], buf[p + 11]]) as usize;
        p += 12;
        if off + len > buf.len() {
            return None;
        }
        entries.push((id, buf[off..off + len].to_vec()));
    }
    Some(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut others = BTreeMap::new();
        others.insert("com.example.a".into(), b"hello".to_vec());
        others.insert("com.example.b".into(), vec![1, 2, 3, 0, 255]);
        let fi = [7u8; 32];
        let blob = build(&fi, &others);
        let (fi2, others2) = parse(&blob).unwrap();
        assert_eq!(fi, fi2);
        assert_eq!(others, others2);
    }
}
