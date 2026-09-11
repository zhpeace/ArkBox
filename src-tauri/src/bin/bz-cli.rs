//! bz-cli —— ArkBox 的「右键/命令行」静默压缩解压辅助二进制（跨平台）。
//!
//! 三种触发方式，最终都复用 `shared::cli_compress` / `cli_extract`（直接调 `archive` 纯函数，无 GUI）：
//! - **macOS Finder 服务（Services）**：由 Info.plist 的 NSServices 声明触发（NSExecutable = bz-cli），
//!   选中文件经系统 pboard 传给供给者方法，静默压到同级 .zip / 解压到同级目录，`open -R` 定位后退出。
//! - **macOS / Windows / Linux 命令行**：`bz-cli compress <path> [path...]` / `bz-cli extract <path> [path...]`，
//!   供注册表 / 文件管理器脚本 / 用户手动调用，不弹窗口。
//! 非 macOS 平台不编译 Services 供给者（`objc` 仅 macOS 依赖），但命令行模式全平台可用。

// objc 0.2 的 msg_send!/sel_impl 宏内部用了 cfg(feature="cargo-clippy")，
// 新版 rustc 会报 unexpected_cfgs；此 lint 对宏展开无效，这里整体放行。
#![allow(unexpected_cfgs)]

/// 平台无关的压缩/解压核心；macOS Services 与 Win/Linux CLI 都调用这里。
mod shared {
    use std::path::{Path, PathBuf};

    use arkbox_lib::archive;

    /// 单文件/目录 → 同级 <原名>.zip；多选 → 公共目录名.zip（落在公共目录同级）；
    /// 跨盘/无公共目录则回退到当前目录用 Archive.zip 命名。
    pub fn compress_dest(paths: &[String]) -> PathBuf {
        if paths.len() == 1 {
            return Path::new(&paths[0]).with_extension("zip");
        }
        let common = common_ancestor_all(paths);
        // 公共祖先只是根目录（如 /a/x + /c/y）或无公共目录时，回退到当前目录，
        // 避免压到 /Archive.zip 这种系统根下的位置。
        if common.as_os_str().is_empty() || common.parent().is_none() {
            return Path::new(".").join("Archive.zip");
        }
        let name = common
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Archive".to_string());
        // zip 以公共目录名命名，放在公共目录的「同级」（即其父目录），避免压进目录自身内部。
        common.parent().unwrap().join(format!("{name}.zip"))
    }

    /// 解压目标：压缩包同级的 <去扩展名> 目录
    pub fn extract_dest(archive_path: &str) -> PathBuf {
        let p = Path::new(archive_path);
        let stem = p
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "extracted".to_string());
        let parent = p.parent().unwrap_or_else(|| Path::new("."));
        parent.join(stem)
    }

    /// 所有路径的最深公共祖先目录（如 /a/b/f1、/a/b/c/f2 → /a/b）。
    fn common_ancestor_all(paths: &[String]) -> PathBuf {
        let mut iter = paths.iter().map(|p| Path::new(p).to_path_buf());
        let mut common = iter.next().unwrap_or_default();
        for p in iter {
            common = common_ancestor(&common, &p);
        }
        common
    }

    fn common_ancestor(a: &Path, b: &Path) -> PathBuf {
        let a_comps: Vec<_> = a.components().collect();
        let b_comps: Vec<_> = b.components().collect();
        let mut i = 0;
        while i < a_comps.len() && i < b_comps.len() && a_comps[i] == b_comps[i] {
            i += 1;
        }
        a_comps[..i].iter().collect()
    }

    pub fn cli_compress(paths: &[String]) {
        if paths.is_empty() {
            notify("未收到任何文件");
            return;
        }
        let dest = compress_dest(paths);
        match archive::compress(paths, &dest.to_string_lossy(), "zip", None, 6, &[], true, None) {
            Ok(_) => reveal(&dest),
            Err(e) => notify(&format!("压缩失败：{e}")),
        }
    }

    pub fn cli_extract(paths: &[String]) {
        if paths.is_empty() {
            notify("未收到任何文件");
            return;
        }
        for archive_path in paths {
            let dest = extract_dest(archive_path);
            match archive::extract(archive_path, &dest.to_string_lossy(), None, None, None) {
                Ok(_) => reveal(&dest),
                Err(e) => notify(&format!("解压失败（{archive_path}）：{e}")),
            }
        }
    }

    /// 在文件管理器中定位结果（按平台选命令）。
    pub fn reveal(path: &Path) {
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("open")
                .args(["-R", &path.to_string_lossy()])
                .status();
        }
        #[cfg(target_os = "windows")]
        {
            let _ = std::process::Command::new("explorer")
                .arg(format!("/select,{}", path.to_string_lossy()))
                .status();
        }
        #[cfg(target_os = "linux")]
        {
            let target = path.parent().unwrap_or(path);
            let _ = std::process::Command::new("xdg-open")
                .arg(target.to_string_lossy().to_string())
                .status();
        }
    }

    /// 出错时给用户提示（macOS 用系统对话框；其他平台先打 stderr，后续可接桌面通知）。
    pub fn notify(msg: &str) {
        #[cfg(target_os = "macos")]
        {
            let escaped = msg.replace('\\', "\\\\").replace('"', "\\\"");
            let script = format!("display alert \"ArkBox\" message \"{escaped}\"");
            let _ = std::process::Command::new("osascript")
                .args(["-e", &script])
                .status();
        }
        #[cfg(not(target_os = "macos"))]
        {
            eprintln!("{msg}");
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn single_file_dest() {
            assert_eq!(compress_dest(&["/a/b.txt".into()]), Path::new("/a/b.zip"));
        }

        #[test]
        fn single_dir_dest() {
            assert_eq!(compress_dest(&["/a/b".into()]), Path::new("/a/b.zip"));
        }

        #[test]
        fn multi_common_dir_named() {
            // /a/b 下的多个文件 → /a/b.zip（以公共目录名命名，落在公共目录的同级）
            let d = compress_dest(&["/a/b/f1.txt".into(), "/a/b/sub/f2.txt".into()]);
            assert_eq!(d, Path::new("/a/b.zip"));
        }

        #[test]
        fn multi_no_common_falls_back() {
            // 跨盘/无公共目录 → Archive.zip
            let d = compress_dest(&["/a/x.txt".into(), "/c/y.txt".into()]);
            assert_eq!(d, Path::new("./Archive.zip"));
        }

        #[test]
        fn extract_dest_strips_ext() {
            assert_eq!(extract_dest("/a/b.zip"), Path::new("/a/b"));
            assert_eq!(extract_dest("/a/b.tar.gz"), Path::new("/a/b.tar"));
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{CStr, CString};
    use std::os::raw::c_char;
    use std::ptr;

    use objc::declare::ClassDecl;
    use objc::runtime::{Class, Object, Sel};
    use objc::{class, msg_send, sel, sel_impl};

    use crate::shared;

    /// 入口：启动最小 NSApplication，注册服务供给者，跑事件循环。
    /// 服务 AppleEvent 到达时由供给者方法处理，处理完调用 NSApp::stop 退出。
    pub fn run() {
        unsafe {
            let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
            // Accessory(1) 不进 Dock；若服务在右键菜单里不出现，改成 0(Regular) 重试。
            let _: () = msg_send![app, setActivationPolicy: 1i64];
            let provider_cls = register_provider_class();
            let provider: *mut Object = msg_send![provider_cls, new];
            let _: () = msg_send![app, setServicesProvider: provider];
            let _: () = msg_send![app, run];
        }
    }

    extern "C" fn arkbox_compress(
        _self: &Object,
        _cmd: Sel,
        pboard: *mut Object,
        _user_data: *mut Object,
        error: *mut Object,
    ) {
        unsafe {
            let paths = read_paths(pboard);
            if paths.is_empty() {
                set_error(error, "未收到任何文件");
                stop_app();
                return;
            }
            shared::cli_compress(&paths);
            stop_app();
        }
    }

    extern "C" fn arkbox_extract(
        _self: &Object,
        _cmd: Sel,
        pboard: *mut Object,
        _user_data: *mut Object,
        error: *mut Object,
    ) {
        unsafe {
            let paths = read_paths(pboard);
            if paths.is_empty() {
                set_error(error, "未收到任何文件");
                stop_app();
                return;
            }
            shared::cli_extract(&paths);
            stop_app();
        }
    }

    fn register_provider_class() -> &'static Class {
        unsafe {
            let mut decl =
                ClassDecl::new("ArkBoxCliServiceProvider", class!(NSObject)).expect("类已存在？");
            decl.add_method(
                sel!(arkboxCompress:userData:error:),
                arkbox_compress
                    as extern "C" fn(&Object, Sel, *mut Object, *mut Object, *mut Object),
            );
            decl.add_method(
                sel!(arkboxExtract:userData:error:),
                arkbox_extract
                    as extern "C" fn(&Object, Sel, *mut Object, *mut Object, *mut Object),
            );
            decl.register()
        }
    }

    unsafe fn read_paths(pboard: *mut Object) -> Vec<String> {
        let type_cstr = CString::new("NSFilenamesPboardType").unwrap();
        let pb_type: *mut Object =
            msg_send![class!(NSString), stringWithUTF8String: type_cstr.as_ptr()];
        let files: *mut Object = msg_send![pboard, propertyListForType: pb_type];
        let mut out = Vec::new();
        if files.is_null() {
            return out;
        }
        let count: usize = msg_send![files, count];
        for i in 0..count {
            let s: *mut Object = msg_send![files, objectAtIndex: i];
            if s.is_null() {
                continue;
            }
            let utf8: *const c_char = msg_send![s, UTF8String];
            if !utf8.is_null() {
                out.push(CStr::from_ptr(utf8).to_string_lossy().into_owned());
            }
        }
        out
    }

    unsafe fn set_error(error: *mut Object, msg: &str) {
        if error.is_null() {
            return;
        }
        let slot = error as *mut *mut Object;
        let cstr = CString::new(msg).unwrap_or_default();
        let ns: *mut Object = msg_send![class!(NSString), stringWithUTF8String: cstr.as_ptr()];
        *slot = ns;
    }

    unsafe fn stop_app() {
        let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        let nil: *mut Object = ptr::null_mut();
        let _: () = msg_send![app, stop: nil];
    }
}

#[cfg(target_os = "macos")]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    // 带 compress/extract 参数时走命令行模式（macOS 也能当 CLI 用）；否则进 Services 事件循环。
    let is_cli = matches!(args.get(1).map(|s| s.as_str()), Some("compress") | Some("extract"));
    if is_cli {
        let mode = args[1].clone();
        let paths: Vec<String> = args.into_iter().skip(2).collect();
        if mode == "compress" {
            shared::cli_compress(&paths);
        } else {
            shared::cli_extract(&paths);
        }
        return;
    }
    macos::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("用法: bz-cli <compress|extract> <path> [path...]");
        std::process::exit(2);
    }
    let mode = args[1].clone();
    let paths: Vec<String> = args.into_iter().skip(2).collect();
    match mode.as_str() {
        "compress" => shared::cli_compress(&paths),
        "extract" => shared::cli_extract(&paths),
        _ => {
            eprintln!("未知模式: {mode}（仅支持 compress / extract）");
            std::process::exit(2);
        }
    }
}
