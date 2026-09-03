//! RAR 只读能力的回归测试。
//!
//! 固件 `tests/data/*.rar` 取自 unrar crate 仓库自带的测试样本（MIT 许可）：
//! - version.rar：单条目、内容加密无
//! - crypted.rar：内容加密（文件名未加密），密码 "unrar"
//!
//! RAR 编码器是专有实现，因此这里只验证读取侧；创建 RAR 应被明确拒绝。

use arkbox_lib::archive::{compress, extract, list_entries, preview_entry, test_archive};
use std::path::PathBuf;

fn sample(name: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
        .to_string_lossy()
        .to_string()
}

#[test]
fn rar_list_and_metadata() {
    let entries = list_entries(&sample("version.rar"), None).expect("应能列出 RAR 条目");
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e.name, "VERSION");
    assert_eq!(e.size, 11);
    assert!(!e.is_dir);
    // UnRAR 不暴露压缩后大小
    assert_eq!(e.compressed_size, None);
    // DOS 时间戳应还原成合理的 Unix 秒（2015 年前后）
    let m = e.modified.expect("应能读出修改时间");
    assert!((1_400_000_000..1_600_000_000).contains(&m), "时间戳异常: {m}");
}

#[test]
fn rar_extract_and_preview() {
    let dir = std::env::temp_dir().join("arkbox-rar-readonly-extract");
    let _ = std::fs::remove_dir_all(&dir);
    let res = extract(
        &sample("version.rar"),
        dir.to_str().unwrap(),
        None,
        None,
        None,
    )
    .expect("应能解压 RAR");
    assert_eq!(res.extracted, 1);
    let content = std::fs::read_to_string(dir.join("VERSION")).expect("应落盘 VERSION");
    assert_eq!(content, "unrar-0.4.0");

    let p = preview_entry(&sample("version.rar"), "VERSION", None, 4096)
        .expect("应能不解压预览");
    assert!(!p.is_binary);
    assert_eq!(p.text.as_deref(), Some("unrar-0.4.0"));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn rar_integrity_test() {
    let t = test_archive(&sample("version.rar"), None).expect("应能校验");
    assert!(t.ok);
    assert_eq!(t.entries.len(), 1);
    assert!(t.entries[0].ok);
}

#[test]
fn rar_encrypted_requires_password() {
    let dir = std::env::temp_dir().join("arkbox-rar-readonly-crypt");
    let _ = std::fs::remove_dir_all(&dir);
    // 文件名未加密，列表本身能过
    assert_eq!(list_entries(&sample("crypted.rar"), None).unwrap().len(), 1);
    // 无密码解压必须报错，且归类到"密码"而不是"文件损坏"
    let err = extract(
        &sample("crypted.rar"),
        dir.to_str().unwrap(),
        None,
        None,
        None,
    )
    .expect_err("无密码不应解压成功");
    assert!(
        err.to_string().contains("加密"),
        "错误应指向密码缺失，实际: {err}"
    );
    // 正确密码（unrar crate 样本使用 "unrar"）应解压成功
    let res = extract(
        &sample("crypted.rar"),
        dir.to_str().unwrap(),
        None,
        Some("unrar"),
        None,
    )
    .expect("正确密码应能解压");
    assert_eq!(res.extracted, 1);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn rar_create_is_rejected() {
    // RAR 编码器未开源授权，创建必须被拒绝且给出可读原因
    let dir = std::env::temp_dir();
    let dest = dir.join("arkbox-should-not-exist.rar");
    let err = compress(
        &[sample("version.rar")],
        dest.to_str().unwrap(),
        "rar",
        None,
        5,
        &[],
        false,
        None,
    )
    .expect_err("创建 RAR 必须失败");
    assert!(
        err.to_string().contains("RAR"),
        "错误应说明 RAR 不可创建，实际: {err}"
    );
    assert!(!dest.exists(), "不应产生任何文件");
}
