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
    let tmp = session.join("edited.bin");
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
