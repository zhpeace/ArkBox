//! 回归测试：修复前加密 zip 的 list/preview/test 因 `by_index`（不带密码）读不到
//! 加密的中央目录文件名，导致即便传了正确密码也报 `Password required`。
//! 修复后这三个函数都改走带密码的 `open_entry`，与 `extract_zip` 行为一致。

use std::fs;
use std::io::Write;

use arkbox_lib::archive::zip::{compress_zip, list_zip, preview_zip, test_zip};

#[test]
fn zip_aes_list_preview_test() {
    let dir = std::env::temp_dir().join("arkbox_zip_aes_test");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    let src = dir.join("secret.txt");
    let mut f = fs::File::create(&src).unwrap();
    f.write_all(b"top secret content line one\ntop secret line two\n")
        .unwrap();
    drop(f);

    let out = dir.join("enc.zip");
    compress_zip(
        &[src.to_str().unwrap().to_string()],
        out.to_str().unwrap(),
        Some("pw123"),
        6,
        &[],
        false,
        None,
    )
    .expect("压缩加密 zip 应成功");

    // 无密码：文件名在中央目录里已加密，应失败（修复前此处即报错）
    let no_pw = list_zip(out.to_str().unwrap(), None);
    assert!(
        no_pw.is_err(),
        "无密码列举加密 zip 应失败，实际: {:?}",
        no_pw.map(|v| v.len())
    );

    // 有密码：能列出
    let listed = list_zip(out.to_str().unwrap(), Some("pw123")).expect("有密码应列出");
    assert_eq!(listed.len(), 1, "应有 1 个条目");
    assert_eq!(listed[0].name, "secret.txt");

    // 有密码：预览能拿到原文
    let pv = preview_zip(out.to_str().unwrap(), "secret.txt", Some("pw123"), 4096)
        .expect("有密码预览应成功");
    let text = pv.text.expect("应为文本预览");
    assert!(text.contains("top secret"), "预览内容应为原文，实际: {text}");

    // 有密码：完整性校验通过
    let ok = test_zip(out.to_str().unwrap(), Some("pw123")).expect("有密码校验应成功");
    assert!(ok.ok, "完整性校验应全部通过");

    // 无密码：校验应失败（同上，读不到加密条目）
    let bad = test_zip(out.to_str().unwrap(), None);
    assert!(bad.is_err(), "无密码校验加密 zip 应失败");

    let _ = fs::remove_dir_all(&dir);
}
