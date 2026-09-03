use std::fs;
use std::path::PathBuf;

use crate::error::{AppError, AppResult};
use crate::model::PresetConfig;

fn presets_dir() -> AppResult<PathBuf> {
    let base = dirs::config_dir()
        .ok_or_else(|| AppError::Config("找不到系统配置目录".into()))?
        .join("arkbox")
        .join("presets");
    fs::create_dir_all(&base)?;
    Ok(base)
}

fn presets_file() -> AppResult<PathBuf> {
    Ok(presets_dir()?.join("presets.json"))
}

pub fn load_presets() -> AppResult<Vec<PresetConfig>> {
    let f = presets_file()?;
    if !f.exists() {
        return Ok(vec![]);
    }
    let s = fs::read_to_string(&f)?;
    if s.trim().is_empty() {
        return Ok(vec![]);
    }
    let v: Vec<PresetConfig> =
        serde_json::from_str(&s).map_err(|e| AppError::Config(e.to_string()))?;
    Ok(v)
}

pub fn save_preset(cfg: &PresetConfig) -> AppResult<()> {
    if cfg.name.trim().is_empty() {
        return Err(AppError::Config("预设名称不能为空".into()));
    }
    let mut v = load_presets()?;
    v.retain(|p| p.name != cfg.name);
    v.push(cfg.clone());
    write(&v)
}

pub fn delete_preset(name: &str) -> AppResult<()> {
    let mut v = load_presets()?;
    v.retain(|p| p.name != name);
    write(&v)
}

fn write(v: &[PresetConfig]) -> AppResult<()> {
    let f = presets_file()?;
    let s = serde_json::to_string_pretty(v).map_err(|e| AppError::Config(e.to_string()))?;
    fs::write(&f, s)?;
    Ok(())
}
