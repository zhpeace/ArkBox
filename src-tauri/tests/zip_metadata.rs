#![cfg(target_os = "macos")]

//! 回归测试：zip 的 macOS 元数据保真
//! - unix 权限（含可执行位）压缩/解压后保留
//! - 符号链接（mode 0o120777）解压后重建为 symlink，目标路径一致
//! - 扩展属性（xattr）通过 `__MACOSX`/AppleDouble 保真还原
//! - 列表不应暴露 `__MACOSX` 元数据条目

use std::fs;
use std::os::darwin::fs::MetadataExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

use arkbox_lib::archive::{compress, extract, list_entries};

#[test]
fn zip_metadata_roundtrip() {
    let dir = std::env::temp_dir().join("arkbox_zip_meta_test");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    let a = dir.join("a.txt");
    fs::write(&a, b"hello").unwrap();
    fs::set_permissions(&a, fs::Permissions::from(PermissionsExt::from_mode(0o755))).unwrap();
    xattr::set(&a, "com.arkbox.test", b"meta-value").unwrap();

    let sub = dir.join("sub");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join("b.txt"), b"world").unwrap();

    let link = dir.join("link.txt");
    symlink("a.txt", &link).unwrap();

    let out_zip = dir.join("out.zip");
    compress(
        &[
            a.to_string_lossy().to_string(),
            sub.to_string_lossy().to_string(),
            link.to_string_lossy().to_string(),
        ],
        out_zip.to_str().unwrap(),
        "zip",
        None,
        6,
        &[],
        true,
        None,
    )
    .unwrap();

    // 列表不应暴露 __MACOSX 元数据条目
    let entries = list_entries(out_zip.to_str().unwrap(), None).unwrap();
    assert!(
        !entries.iter().any(|e| e.name.starts_with("__MACOSX/")),
        "列表不应含 __MACOSX"
    );

    let out = dir.join("extracted");
    let res = extract(out_zip.to_str().unwrap(), out.to_str().unwrap(), None, None, None).unwrap();
    // a.txt / sub/b.txt / link.txt 共 3 个非目录条目
    assert_eq!(res.extracted, 3, "应解压 3 个文件");

    // 常规文件：内容 + 权限 + xattr
    let a_out = out.join("a.txt");
    assert_eq!(fs::read(&a_out).unwrap(), b"hello");
    let mode = fs::metadata(&a_out).unwrap().st_mode() & 0o777;
    assert_eq!(mode, 0o755, "unix 权限应保留");
    assert_eq!(
        xattr::get(&a_out, "com.arkbox.test")
            .unwrap()
            .unwrap(),
        b"meta-value",
        "xattr 应还原"
    );

    // 符号链接：重建为 symlink 且目标一致
    assert!(
        fs::symlink_metadata(out.join("link.txt"))
            .unwrap()
            .file_type()
            .is_symlink(),
        "应重建为符号链接"
    );
    assert_eq!(fs::read_link(out.join("link.txt")).unwrap(), Path::new("a.txt"));

    // 子目录文件
    assert_eq!(fs::read(out.join("sub").join("b.txt")).unwrap(), b"world");

    let _ = fs::remove_dir_all(&dir);
}
