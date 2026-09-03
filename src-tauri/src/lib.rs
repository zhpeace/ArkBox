pub mod archive;
mod commands;
mod crypto;
mod edit;
mod error;
pub mod model;
mod presets;

use commands::*;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};

/// 文件关联打开 / 拖放待加载的压缩包路径队列（冷启动也能被前端拉取）
pub struct PendingOpen(pub Arc<Mutex<Vec<String>>>);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let pending = PendingOpen(Arc::new(Mutex::new(Vec::new())));
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .manage(pending)
        .invoke_handler(tauri::generate_handler![
            compress,
            extract,
            list_entries,
            preview_entry,
            test_archive,
            save_preset,
            load_presets,
            delete_preset,
            begin_edit_entry,
            commit_edit,
            cancel_edit,
            take_pending_open
        ])
        .build(tauri::generate_context!())
        .expect("error while building ArkBox");

    app.run(|app_handle, event| {
        if let tauri::RunEvent::Opened { urls } = event {
            let mut paths: Vec<String> = Vec::new();
            for u in urls {
                if let Ok(p) = u.to_file_path() {
                    paths.push(p.to_string_lossy().to_string());
                }
            }
            if !paths.is_empty() {
                if let Some(state) = app_handle.try_state::<PendingOpen>() {
                    state.0.lock().unwrap().extend(paths.clone());
                }
                let _ = app_handle.emit("opened-files", &paths);
            }
        }
    });
}

#[cfg(test)]
mod smoke {
    use super::*;
    use std::fs;

    fn stage(dir: &std::path::Path, name: &str, content: &[u8]) -> std::path::PathBuf {
        let p = dir.join(name);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&p, content).unwrap();
        p
    }

    fn roundtrip(fmt: &str) {
        let dir = std::env::temp_dir().join(format!("bz_smoke_{}", fmt.replace('.', "_")));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let f1 = stage(&dir, "a.txt", b"hello world");
        let sub = stage(&dir, "sub/b.txt", b"nested content");
        let out = dir.join(format!("out.{}", ext(fmt)));
        let info = archive::compress(
            &[f1.to_string_lossy().to_string(), sub.to_string_lossy().to_string()],
            out.to_str().unwrap(),
            fmt,
            None,
            6,
            &[],
            true,
            None,
        )
        .unwrap();
        assert!(info.entry_count >= 2, "entry count for {fmt}");

        let entries = archive::list_entries(out.to_str().unwrap(), None).unwrap();
        assert!(entries.iter().any(|e| e.name.ends_with("a.txt")));

        let dest = dir.join("extracted");
        let res = archive::extract(out.to_str().unwrap(), dest.to_str().unwrap(), None, None, None).unwrap();
        assert!(res.extracted >= 2, "extracted count for {fmt}");
        assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"hello world");

        let _ = fs::remove_dir_all(&dir);
    }

    fn ext(fmt: &str) -> &str {
        match fmt {
            "zip" => "zip",
            "7z" => "7z",
            "targz" => "tar.gz",
            "tarbz" => "tar.bz2",
            "tarxz" => "tar.xz",
            "tarzstd" => "tar.zst",
            _ => "bin",
        }
    }

    #[test]
    fn zip_roundtrip() {
        roundtrip("zip");
    }
    #[test]
    fn sevenz_roundtrip() {
        roundtrip("7z");
    }
    #[test]
    fn targz_roundtrip() {
        roundtrip("targz");
    }
    #[test]
    fn tarxz_roundtrip() {
        roundtrip("tarxz");
    }

    #[test]
    fn zip_aes_roundtrip() {
        let dir = std::env::temp_dir().join("bz_smoke_aes");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let f = stage(&dir, "secret.txt", b"top secret");
        let out = dir.join("s.zip");
        archive::compress(
            &[f.to_string_lossy().to_string()],
            out.to_str().unwrap(),
            "zip",
            Some("pw123"),
            6,
            &[],
            true,
            None,
        )
        .unwrap();
        // 无密码解压应失败或无内容
        let dest = dir.join("ext");
        let r = archive::extract(out.to_str().unwrap(), dest.to_str().unwrap(), None, None, None);
        assert!(r.is_err(), "无密码应解压失败");
        // 正确密码解压成功
        let dest2 = dir.join("ext2");
        let res = archive::extract(out.to_str().unwrap(), dest2.to_str().unwrap(), None, Some("pw123"), None).unwrap();
        assert_eq!(res.extracted, 1);
        assert_eq!(fs::read(dest2.join("secret.txt")).unwrap(), b"top secret");
        let _ = fs::remove_dir_all(&dir);
    }

    /// 7z 加密：sevenz-rust 需开启 aes256 feature，并用 [AES, LZMA2] 方法链。
    /// 加密后文件头也被加密，因此无密码连文件名都列不出来。
    #[test]
    fn sevenz_aes_roundtrip() {
        let dir = std::env::temp_dir().join("bz_smoke_7z_aes");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let f = stage(&dir, "secret.txt", b"top secret 7z");
        let out = dir.join("s.7z");

        let start = std::time::Instant::now();
        archive::compress(
            &[f.to_string_lossy().to_string()],
            out.to_str().unwrap(),
            "7z",
            Some("pw123"),
            6,
            &[],
            true,
            None,
        )
        .unwrap();
        println!(
            "7z 加密压缩耗时: {:?}（含 2^19 次 SHA-256 派生）",
            start.elapsed()
        );

        // 文件头已加密：无密码列表必须失败
        assert!(
            archive::list_entries(out.to_str().unwrap(), None).is_err(),
            "无密码不应能列出加密 7z 的内容"
        );
        // 注意：sevenz-rust 的文件头解密不校验密码，任何非空密码都能列出文件名，
        // 因此这里不能断言"错误密码列表失败"——内容本身仍是安全的（下面 extract 会失败）。
        let entries = archive::list_entries(out.to_str().unwrap(), Some("pw123")).unwrap();
        assert!(entries.iter().any(|e| e.name.ends_with("secret.txt")));

        // 无密码解压失败
        let dest = dir.join("ext");
        assert!(
            archive::extract(out.to_str().unwrap(), dest.to_str().unwrap(), None, None, None)
                .is_err(),
            "无密码应解压失败"
        );
        // 错误密码：内容必须取不出来（解压报密码错误）
        let destw = dir.join("ext_wrong");
        let err = archive::extract(
            out.to_str().unwrap(),
            destw.to_str().unwrap(),
            None,
            Some("wrong"),
            None,
        )
        .expect_err("错误密码应解压失败");
        assert!(
            err.to_string().contains("密码"),
            "错误密码应归到密码类错误，实际: {err}"
        );
        // 正确密码解压成功且内容一致
        let dest2 = dir.join("ext2");
        let res = archive::extract(
            out.to_str().unwrap(),
            dest2.to_str().unwrap(),
            None,
            Some("pw123"),
            None,
        )
        .unwrap();
        assert_eq!(res.extracted, 1);
        assert_eq!(
            fs::read(dest2.join("secret.txt")).unwrap(),
            b"top secret 7z"
        );
        // 带密码的完整性校验应通过
        assert!(archive::test_archive(out.to_str().unwrap(), Some("pw123"))
            .unwrap()
            .ok);
        // 错误密码的完整性校验绝不能报“通过”——否则压缩后的自动校验就是假绿灯。
        // 由于文件头解密不校验密码，错误密码照样能列出条目，所以这里必须真读内容。
        match archive::test_archive(out.to_str().unwrap(), Some("wrong")) {
            Ok(t) => assert!(!t.ok, "错误密码的完整性检测不应通过（假绿灯）: {t:?}"),
            Err(e) => assert!(e.to_string().contains("密码"), "错误密码应归到密码类错误，实际: {e}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn preview_text() {
        let dir = std::env::temp_dir().join("bz_smoke_preview");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let f = stage(&dir, "note.txt", b"line one\nline two");
        let out = dir.join("p.zip");
        archive::compress(&[f.to_string_lossy().to_string()], out.to_str().unwrap(), "zip", None, 6, &[], true, None).unwrap();
        let pv = archive::preview_entry(out.to_str().unwrap(), "note.txt", None, 1000).unwrap();
        assert!(!pv.is_binary);
        assert!(pv.text.unwrap().contains("line one"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn single_file_roundtrip() {
        for fmt in ["gz", "bz2", "xz", "zst"] {
            let dir = std::env::temp_dir().join(format!("bz_smoke_single_{}", fmt));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let f = stage(&dir, "a.txt", b"single file payload 12345");
            let out = dir.join(format!("out.{}", fmt));
            let info = archive::compress(
                &[f.to_string_lossy().to_string()],
                out.to_str().unwrap(),
                fmt,
                None,
                6,
                &[],
                true,
                None,
            )
            .unwrap();
            assert_eq!(info.entry_count, 1, "entry count for {fmt}");
            let dest = dir.join("extracted");
            let res = archive::extract(out.to_str().unwrap(), dest.to_str().unwrap(), None, None, None).unwrap();
            assert_eq!(res.extracted, 1, "extracted for {fmt}");
            // 派生名 = 压缩包文件名去扩展名 -> "out"
            assert_eq!(
                fs::read(dest.join("out")).unwrap(),
                b"single file payload 12345",
                "content for {fmt}"
            );
            let _ = fs::remove_dir_all(&dir);
        }
    }
}
