use std::fs;
use std::path::PathBuf;

use uuid::Uuid;

use crate::archive;
use crate::error::{AppError, AppResult};
use crate::model::{ArchiveFormat, EditHandle};

fn edit_root() -> AppResult<PathBuf> {
    let dir = std::env::temp_dir().join("arkbox-edit");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn sanitize_name(name: &str) -> String {
    name.split('/')
        .filter(|s| !s.is_empty() && *s != "." && *s != "..")
        .collect::<Vec<_>>()
        .join("/")
}

/// 仅把目标条目解压到临时文件，返回编辑句柄。
pub fn begin_edit_entry(
    archive: &str,
    entry: &str,
    password: Option<&str>,
) -> AppResult<EditHandle> {
    let root = edit_root()?;
    let session = root.join(Uuid::new_v4().to_string());
    fs::create_dir_all(&session)?;

    archive::extract(
        archive,
        session.to_str().unwrap(),
        Some(&[entry.to_string()]),
        password,
        None,
    )?;

    let extracted = session.join(sanitize_name(entry));
    if !extracted.exists() {
        let _ = fs::remove_dir_all(&session);
        return Err(AppError::Archive(format!(
            "条目未解压出可编辑文件: {entry}"
        )));
    }
    // 临时文件保留原文件名（含扩展名）：macOS/Windows 按扩展名选择默认打开程序，
    // 固定叫 edited.bin 会让 txt/json/xml 找不到关联编辑器。
    let base = entry.rsplit('/').next().unwrap_or("edited.bin");
    let base = if base.is_empty() { "edited.bin" } else { base };
    let tmp = session.join(sanitize_name(base));
    fs::rename(&extracted, &tmp).map_err(|e| AppError::Io(e))?;

    Ok(EditHandle {
        handle_id: session
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string(),
        temp_path: tmp.to_string_lossy().to_string(),
        entry_name: entry.to_string(),
    })
}

/// 用编辑后的文件覆盖原条目并重打包整个压缩包。
pub fn commit_edit(
    handle: &EditHandle,
    archive: &str,
    password: Option<&str>,
) -> AppResult<()> {
    let fmt = ArchiveFormat::from_ext(archive)
        .ok_or_else(|| AppError::UnsupportedFormat(format!("无法识别: {archive}")))?;
    let root = edit_root()?;
    let session = root.join(&handle.handle_id);
    let work = root.join(format!("{}-rebuild", handle.handle_id));
    fs::create_dir_all(&work)?;

    // 全量解压到 work
    archive::extract(archive, work.to_str().unwrap(), None, password, None)?;

    // 用编辑后的文件替换目标条目
    let target = work.join(sanitize_name(&handle.entry_name));
    fs::copy(&handle.temp_path, &target).map_err(|e| AppError::Io(e))?;

    // 以 work 的直接子项为根重新压缩，覆盖原包
    let mut paths: Vec<String> = Vec::new();
    for child in fs::read_dir(&work)? {
        let child = child?;
        paths.push(child.path().to_string_lossy().to_string());
    }
    if paths.is_empty() {
        return Err(AppError::Archive("重打包时未找到任何条目".into()));
    }
    archive::compress(&paths, archive, fmt.as_str(), password, 6, &[], true, None)?;

    cleanup(&session, &work);
    Ok(())
}

pub fn cancel_edit(handle: EditHandle) -> AppResult<()> {
    let root = edit_root()?;
    let session = root.join(&handle.handle_id);
    let work = root.join(format!("{}-rebuild", handle.handle_id));
    cleanup(&session, &work);
    Ok(())
}

fn cleanup(session: &PathBuf, work: &PathBuf) {
    let _ = fs::remove_dir_all(session);
    let _ = fs::remove_dir_all(work);
}

/// 从压缩包内删除若干条目（含其嵌套子条目），重打包覆盖原包。
/// 复用 commit_edit 的「全量解包到 work -> 改 -> archive::compress 重打包」路径。
pub fn delete_entries(
    archive: &str,
    entries: &[String],
    password: Option<&str>,
) -> AppResult<()> {
    let fmt = ArchiveFormat::from_ext(archive)
        .ok_or_else(|| AppError::UnsupportedFormat(format!("无法识别: {archive}")))?;
    if fmt == ArchiveFormat::Rar {
        return Err(AppError::UnsupportedFormat(
            "RAR 为只读格式，不支持增删改".into(),
        ));
    }
    let root = edit_root()?;
    let session_id = Uuid::new_v4().to_string();
    let session = root.join(&session_id);
    let work = root.join(format!("{session_id}-rebuild"));
    fs::create_dir_all(&work)?;

    // 全量解包到 work
    archive::extract(archive, work.to_str().unwrap(), None, password, None)?;

    // 删除指定条目（文件或目录，目录连带子条目整体移除）
    for e in entries {
        let target = work.join(sanitize_name(e));
        if target.exists() {
            if target.is_dir() {
                fs::remove_dir_all(&target)?;
            } else {
                fs::remove_file(&target)?;
            }
        }
    }

    // 以 work 顶层子项重打包覆盖原包
    let mut paths: Vec<String> = Vec::new();
    for child in fs::read_dir(&work)? {
        let child = child?;
        paths.push(child.path().to_string_lossy().to_string());
    }
    if paths.is_empty() {
        return Err(AppError::Archive("删除后压缩包已无条目".into()));
    }
    archive::compress(&paths, archive, fmt.as_str(), password, 6, &[], true, None)?;

    cleanup(&session, &work);
    Ok(())
}

/// 重命名压缩包内单个条目。若其为目录，整棵目录（含所有子条目）一并改名。
pub fn rename_entry(
    archive: &str,
    old_name: &str,
    new_name: &str,
    password: Option<&str>,
) -> AppResult<()> {
    let fmt = ArchiveFormat::from_ext(archive)
        .ok_or_else(|| AppError::UnsupportedFormat(format!("无法识别: {archive}")))?;
    if fmt == ArchiveFormat::Rar {
        return Err(AppError::UnsupportedFormat(
            "RAR 为只读格式，不支持增删改".into(),
        ));
    }
    let root = edit_root()?;
    let session_id = Uuid::new_v4().to_string();
    let session = root.join(&session_id);
    let work = root.join(format!("{session_id}-rebuild"));
    fs::create_dir_all(&work)?;

    archive::extract(archive, work.to_str().unwrap(), None, password, None)?;

    let old = work.join(sanitize_name(old_name));
    let new = work.join(sanitize_name(new_name));
    if !old.exists() {
        cleanup(&session, &work);
        return Err(AppError::Archive(format!("条目不存在，无法重命名: {old_name}")));
    }
    if new.exists() {
        if new.is_dir() {
            fs::remove_dir_all(&new)?;
        } else {
            fs::remove_file(&new)?;
        }
    }
    fs::rename(&old, &new).map_err(AppError::Io)?;

    let mut paths: Vec<String> = Vec::new();
    for child in fs::read_dir(&work)? {
        let child = child?;
        paths.push(child.path().to_string_lossy().to_string());
    }
    archive::compress(&paths, archive, fmt.as_str(), password, 6, &[], true, None)?;

    cleanup(&session, &work);
    Ok(())
}

/// 把若干本地文件添加进压缩包（置于根目录，重名覆盖），重打包覆盖原包。
pub fn add_entries(
    archive: &str,
    new_files: &[String],
    password: Option<&str>,
) -> AppResult<()> {
    let fmt = ArchiveFormat::from_ext(archive)
        .ok_or_else(|| AppError::UnsupportedFormat(format!("无法识别: {archive}")))?;
    if fmt == ArchiveFormat::Rar {
        return Err(AppError::UnsupportedFormat(
            "RAR 为只读格式，不支持增删改".into(),
        ));
    }
    // 单流格式（gz/bz2/xz/zst）只容纳单个文件，添加多个条目无意义
    if matches!(
        fmt,
        ArchiveFormat::Gz | ArchiveFormat::Bz2 | ArchiveFormat::Xz | ArchiveFormat::Zstd
    ) {
        return Err(AppError::UnsupportedFormat(format!(
            "{} 为单文件流格式，不支持添加多个条目",
            fmt.as_str()
        )));
    }
    if new_files.is_empty() {
        return Ok(());
    }
    let root = edit_root()?;
    let session_id = Uuid::new_v4().to_string();
    let session = root.join(&session_id);
    let work = root.join(format!("{session_id}-rebuild"));
    fs::create_dir_all(&work)?;

    archive::extract(archive, work.to_str().unwrap(), None, password, None)?;

    for f in new_files {
        let src = PathBuf::from(f);
        if !src.exists() {
            cleanup(&session, &work);
            return Err(AppError::Archive(format!("待添加文件不存在: {f}")));
        }
        let base = src
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .ok_or_else(|| AppError::Archive(format!("无法解析文件名: {f}")))?;
        let dst = work.join(&base);
        fs::copy(&src, &dst).map_err(AppError::Io)?;
    }

    let mut paths: Vec<String> = Vec::new();
    for child in fs::read_dir(&work)? {
        let child = child?;
        paths.push(child.path().to_string_lossy().to_string());
    }
    if paths.is_empty() {
        return Err(AppError::Archive("添加后压缩包已无条目".into()));
    }
    archive::compress(&paths, archive, fmt.as_str(), password, 6, &[], true, None)?;

    cleanup(&session, &work);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn stage(dir: &std::path::Path, rel: &str, content: &[u8]) -> std::path::PathBuf {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = fs::File::create(&p).unwrap();
        f.write_all(content).unwrap();
        p
    }

    fn build_zip(dir: &std::path::Path) -> std::path::PathBuf {
        stage(dir, "a.txt", b"alpha");
        stage(dir, "sub/b.txt", b"bravo");
        stage(dir, "c.txt", b"charlie");
        let out = dir.join("out.zip");
        let paths = vec![
            dir.join("a.txt").to_string_lossy().to_string(),
            dir.join("sub").to_string_lossy().to_string(),
            dir.join("c.txt").to_string_lossy().to_string(),
        ];
        archive::compress(&paths, out.to_str().unwrap(), "zip", None, 6, &[], true, None)
            .unwrap();
        out
    }

    fn names(out: &std::path::Path) -> Vec<String> {
        archive::list_entries(out.to_str().unwrap(), None)
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect()
    }

    #[test]
    fn zip_delete_rename_add_roundtrip() {
        let dir = std::env::temp_dir().join("bz_edit_zip");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let out = build_zip(&dir);

        let mut n = names(&out);
        assert!(n.contains(&"a.txt".to_string()), "初始应有 a.txt: {n:?}");
        assert!(
            n.iter().any(|x| x.ends_with("sub/b.txt")),
            "初始应有 sub/b.txt: {n:?}"
        );
        assert!(n.contains(&"c.txt".to_string()), "初始应有 c.txt: {n:?}");

        // 删除目录 sub（连带子条目一并移除）
        delete_entries(out.to_str().unwrap(), &["sub".to_string()], None).unwrap();
        n = names(&out);
        assert!(
            !n.iter().any(|x| x.starts_with("sub")),
            "删除后不应再有 sub: {n:?}"
        );
        assert!(n.contains(&"a.txt".to_string()));
        assert!(n.contains(&"c.txt".to_string()));

        // 重命名 c.txt -> renamed.txt
        rename_entry(out.to_str().unwrap(), "c.txt", "renamed.txt", None).unwrap();
        n = names(&out);
        assert!(
            n.contains(&"renamed.txt".to_string()),
            "重命名后应出现 renamed.txt: {n:?}"
        );
        assert!(
            !n.contains(&"c.txt".to_string()),
            "c.txt 应已消失: {n:?}"
        );

        // 添加本地文件 d.txt
        let d = stage(&dir, "d.txt", b"delta");
        add_entries(out.to_str().unwrap(), &[d.to_string_lossy().to_string()], None).unwrap();
        n = names(&out);
        assert!(n.contains(&"d.txt".to_string()), "添加后应有 d.txt: {n:?}");

        // 解压校验全部内容正确
        let dest = dir.join("ext");
        archive::extract(out.to_str().unwrap(), dest.to_str().unwrap(), None, None, None)
            .unwrap();
        assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"alpha");
        assert_eq!(fs::read(dest.join("renamed.txt")).unwrap(), b"charlie");
        assert_eq!(fs::read(dest.join("d.txt")).unwrap(), b"delta");
        assert!(!dest.join("sub").exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sevenz_delete_add_smoke() {
        let dir = std::env::temp_dir().join("bz_edit_7z");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        stage(&dir, "a.txt", b"alpha");
        stage(&dir, "c.txt", b"charlie");
        let out = dir.join("out.7z");
        let paths = vec![
            dir.join("a.txt").to_string_lossy().to_string(),
            dir.join("c.txt").to_string_lossy().to_string(),
        ];
        archive::compress(&paths, out.to_str().unwrap(), "7z", None, 6, &[], true, None)
            .unwrap();

        delete_entries(out.to_str().unwrap(), &["c.txt".to_string()], None).unwrap();
        let n = names(&out);
        assert!(n.contains(&"a.txt".to_string()));
        assert!(!n.contains(&"c.txt".to_string()));

        let d = stage(&dir, "d.txt", b"delta");
        add_entries(out.to_str().unwrap(), &[d.to_string_lossy().to_string()], None).unwrap();
        let n = names(&out);
        assert!(n.contains(&"d.txt".to_string()), "7z 添加后应有 d.txt: {n:?}");

        let _ = fs::remove_dir_all(&dir);
    }
}
