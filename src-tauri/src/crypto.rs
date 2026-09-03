/// 密码相关小工具（当前仅用于日志脱敏，后续可扩展强度校验）。

/// 把密码替换成固定长度的掩码，避免日志里泄露明文。
pub fn mask_password(pw: &str) -> String {
    if pw.is_empty() {
        "<空>".into()
    } else {
        "*".repeat(pw.chars().count().min(12))
    }
}
