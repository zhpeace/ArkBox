#[cfg(target_os = "macos")]
mod imp {
    use crate::error::{AppError, AppResult};

    const SERVICE: &str = "arkbox";

    /// 把某个压缩包的密码存入 macOS 钥匙串（以压缩包完整路径作为账户名）。
    /// `-U` 表示若已存在则更新；写入失败（如用户取消授权）仅返回错误，不影响压缩主流程。
    pub fn save_archive_password(archive: &str, password: &str) -> AppResult<()> {
        let status = std::process::Command::new("security")
            .args([
                "add-generic-password",
                "-a",
                archive,
                "-s",
                SERVICE,
                "-w",
                password,
                "-U",
            ])
            .status()
            .map_err(|e| AppError::Keychain(e.to_string()))?;
        if !status.success() {
            return Err(AppError::Keychain(format!(
                "保存密码到钥匙串失败 (exit {})",
                status.code().unwrap_or(-1)
            )));
        }
        Ok(())
    }

    /// 读取某压缩包在钥匙串中保存的密码；不存在或读取失败返回 None。
    pub fn load_archive_password(archive: &str) -> Option<String> {
        let out = std::process::Command::new("security")
            .args(["find-generic-password", "-a", archive, "-s", SERVICE, "-w"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }
}

// 非 macOS 平台：钥匙串不可用，密码记忆降级为「不持久化」，由调用方回退到手动输入。
#[cfg(not(target_os = "macos"))]
mod imp {
    use crate::error::AppResult;

    pub fn save_archive_password(_archive: &str, _password: &str) -> AppResult<()> {
        Ok(())
    }

    pub fn load_archive_password(_archive: &str) -> Option<String> {
        None
    }
}

pub use imp::*;
