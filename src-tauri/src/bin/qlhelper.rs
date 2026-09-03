//! bz-qlhelper —— Quick Look 预览辅助二进制。
//!
//! 用法：bz-qlhelper [--password <pw>] <archive-path>
//! 把压缩包内容渲染成「自包含 HTML」输出到 stdout，供 QL 生成器直接喂给
//! QLPreviewRequestSetDataRepresentation(kUTTypeHTML, ...)。
//! 出错时打印一张错误页并退出码非 0。

use std::env;
use std::path::Path;

use arkbox_lib::archive;
use arkbox_lib::model::EntryInfo;

fn human_size(n: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB", "PB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} {}", UNITS[0])
    } else {
        format!("{:.1} {}", v, UNITS[i])
    }
}

fn fmt_modified(e: &EntryInfo) -> String {
    match e.modified {
        Some(ts) => format_unix_utc(ts),
        None => "—".to_string(),
    }
}

/// 把 unix 秒转成 UTC 的 "YYYY-MM-DD HH:MM"（仅用于展示，忽略闰秒）。
fn format_unix_utc(ts: u64) -> String {
    let secs_of_day = (ts % 86400) as u32;
    let mut days = ts / 86400;
    let mut year = 1970i64;
    loop {
        let leap = is_leap(year);
        let ydays = if leap { 366 } else { 365 };
        if days >= ydays {
            days -= ydays;
            year += 1;
        } else {
            break;
        }
    }
    let (month, mday) = day_of_year_to_md(year, days as u32);
    let hh = secs_of_day / 3600;
    let mm = (secs_of_day % 3600) / 60;
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        year, month, mday, hh, mm
    )
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// days: 年内第几天（0-based）→ (月 1-12, 日 1-31)
fn day_of_year_to_md(year: i64, days: u32) -> (u32, u32) {
    let month_days: [u32; 12] = if is_leap(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut rem = days;
    for (i, &md) in month_days.iter().enumerate() {
        if rem < md {
            return ((i + 1) as u32, rem + 1);
        }
        rem -= md;
    }
    (12, 31)
}

fn icon_glyph(name: &str, is_dir: bool) -> &'static str {
    if is_dir {
        return "📁";
    }
    let lower = name.to_ascii_lowercase();
    match lower.rsplit('.').next().unwrap_or("") {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" | "ico" => "🖼️",
        "txt" | "md" | "log" | "csv" | "json" | "xml" | "yaml" | "yml" | "toml" | "rs" | "js"
        | "ts" | "vue" | "py" | "c" | "h" | "cpp" | "go" | "java" => "📄",
        "mp3" | "wav" | "flac" | "m4a" | "ogg" => "🎵",
        "mp4" | "mov" | "mkv" | "avi" | "webm" => "🎬",
        "zip" | "7z" | "gz" | "tgz" | "bz2" | "tbz" | "xz" | "txz" | "zst" | "tar" | "rar"
        | "lzh" => "🗜️",
        "pdf" => "📕",
        "doc" | "docx" | "pages" => "📘",
        "xls" | "xlsx" | "numbers" => "📗",
        "ppt" | "pptx" | "key" => "📙",
        "app" | "exe" | "dmg" | "pkg" => "⚙️",
        _ => "📄",
    }
}

fn render(archive: &str, entries: &[EntryInfo], encrypted: bool) -> String {
    let file_name = Path::new(archive)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(archive);

    let total: u64 = entries.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();
    let files = entries.iter().filter(|e| !e.is_dir).count();
    let dirs = entries.iter().filter(|e| e.is_dir).count();

    let mut rows = String::new();
    for e in entries {
        let depth = e.name.matches('/').count();
        let pad = depth * 16;
        let glyph = icon_glyph(&e.name, e.is_dir);
        let size = if e.is_dir {
            "—".to_string()
        } else {
            human_size(e.size)
        };
        let method = e.method.as_deref().unwrap_or("—");
        rows.push_str(&format!(
            "<tr>\
              <td class=\"ic\">{glyph}</td>\
              <td style=\"padding-left:{pad}px\">{name}</td>\
              <td class=\"num\">{size}</td>\
              <td class=\"mut\">{method}</td>\
              <td class=\"mut\">{mod}</td>\
            </tr>",
            name = html_escape(&e.name),
            mod = html_escape(&fmt_modified(e)),
        ));
    }

    let lock = if encrypted {
        "<span class=\"lock\">🔒 已加密</span>"
    } else {
        ""
    };

    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
<style>\
  * {{ box-sizing: border-box; }} \
  body {{ margin:0; font-family:-apple-system,BlinkMacSystemFont,\"SF Pro Text\",\"Helvetica Neue\",Arial,sans-serif; \
         color:#1d1d1f; background:#fff; }} \
  .wrap {{ padding:18px 22px; }} \
  .head {{ display:flex; align-items:baseline; gap:10px; border-bottom:1px solid #e6e6e6; padding-bottom:10px; }} \
  .title {{ font-size:17px; font-weight:600; }} \
  .meta {{ font-size:12px; color:#86868b; margin-left:auto; }} \
  .lock {{ color:#b25000; background:#fff3e0; border:1px solid #ffd8a8; border-radius:6px; \
          padding:2px 8px; font-size:12px; font-weight:500; }} \
  table {{ width:100%; border-collapse:collapse; margin-top:10px; font-size:13px; }} \
  tr {{ border-bottom:1px solid #f2f2f2; }} \
  td {{ padding:5px 8px; vertical-align:middle; }} \
  td.ic {{ width:26px; font-size:14px; }} \
  td.num {{ text-align:right; color:#1d1d1f; font-variant-numeric:tabular-nums; white-space:nowrap; }} \
  td.mut {{ color:#86868b; font-size:11px; white-space:nowrap; }} \
  .sum {{ font-size:12px; color:#86868b; margin-top:10px; }} \
</style></head><body><div class=\"wrap\">\
  <div class=\"head\"><span class=\"title\">{title}</span>{lock}\
    <span class=\"meta\">{files} 个文件 · {dirs} 个文件夹 · 共 {total}</span></div>\
  <table>{rows}</table>\
  <div class=\"sum\">ArkBox · Quick Look 预览</div>\
</div></body></html>",
        title = html_escape(file_name),
        rows = rows,
        files = files,
        dirs = dirs,
        total = human_size(total),
    )
}

fn render_error(archive: &str, msg: &str) -> String {
    let file_name = Path::new(archive)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(archive);
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
<style>body{{font-family:-apple-system,sans-serif;padding:24px;color:#1d1d1f;}} \
.err{{color:#b00;font-weight:600;}} .mut{{color:#86868b;font-size:12px;}}</style>\
</head><body><div class=\"title\">{title}</div>\
<div class=\"err\" style=\"margin-top:12px\">无法预览此压缩包</div>\
<div class=\"mut\" style=\"margin-top:6px\">{msg}</div>\
<div class=\"mut\" style=\"margin-top:18px\">ArkBox · Quick Look 预览</div>\
</body></html>",
        title = html_escape(file_name),
        msg = html_escape(msg),
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut password: Option<String> = None;
    let mut archive: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        let a = &args[i];
        if a == "--password" || a == "-p" {
            if i + 1 < args.len() {
                password = Some(args[i + 1].clone());
                i += 2;
                continue;
            }
        } else if let Some(pw) = a.strip_prefix("--password=") {
            password = Some(pw.to_string());
            i += 1;
            continue;
        } else if archive.is_none() {
            archive = Some(a.clone());
        }
        i += 1;
    }

    let archive = match archive {
        Some(a) => a,
        None => {
            eprintln!("usage: bz-qlhelper [--password <pw>] <archive-path>");
            std::process::exit(2);
        }
    };

    // 先尝试列目录（带密码），失败则尝试无密码，判断是否为加密包。
    let result = match &password {
        Some(pw) => archive::list_entries(&archive, Some(pw)),
        None => archive::list_entries(&archive, None),
    };

    match result {
        Ok(entries) => {
            let html = render(&archive, &entries, false);
            print!("{html}");
        }
        Err(first_err) => {
            // 未给密码时失败，再试一次判断是否加密
            if password.is_none() {
                if let Ok(entries) = archive::list_entries(&archive, Some("")) {
                    let html = render(&archive, &entries, true);
                    print!("{html}");
                    return;
                }
            }
            let html = render_error(&archive, &first_err.to_string());
            print!("{html}");
            std::process::exit(1);
        }
    }
}
