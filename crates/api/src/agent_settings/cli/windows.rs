use std::path::{Path, PathBuf};
use std::process::Command;

use failure::Failure;

use super::AgentCliState;
use crate::CallError;

pub(in crate::agent_settings) fn location(executable: &Path) -> Result<PathBuf, CallError> {
    executable
        .parent()
        .map(Path::to_owned)
        .ok_or_else(|| CallError::internal("CLI has no parent directory"))
}

pub(in crate::agent_settings) fn state(executable: &Path) -> Result<AgentCliState, CallError> {
    Ok(if run(executable, "status")? {
        AgentCliState::Installed
    } else {
        AgentCliState::NotInstalled
    })
}

pub(in crate::agent_settings) fn install(executable: &Path) -> Result<(), CallError> {
    run(executable, "install").map(|_| ())
}

pub(in crate::agent_settings) fn uninstall(executable: &Path) -> Result<(), CallError> {
    run(executable, "uninstall").map(|_| ())
}

fn run(executable: &Path, action: &str) -> Result<bool, CallError> {
    use std::os::windows::process::CommandExt;

    let root = std::env::var_os("SystemRoot").ok_or_else(|| {
        CallError::external(
            Failure::new("找不到 Windows PowerShell，命令行工具未能更改"),
            "SystemRoot is not set",
        )
    })?;
    let output =
        Command::new(PathBuf::from(root).join("System32/WindowsPowerShell/v1.0/powershell.exe"))
            .creation_flags(0x0800_0000)
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                include_str!("windows.ps1"),
            ])
            .env("AUTOEMAIL_CLI_DIRECTORY", location(executable)?)
            .env("AUTOEMAIL_CLI_ACTION", action)
            .output()
            .map_err(|e| {
                CallError::external(
                    Failure::unrecognized(
                        "无法运行 Windows PowerShell，命令行工具未能更改",
                        &e.to_string(),
                    ),
                    e,
                )
            })?;
    if !output.status.success() {
        let words = String::from_utf8_lossy(&output.stderr);
        return Err(CallError::external(
            Failure::unrecognized(
                "无法更新当前用户的 PATH。请检查用户环境变量的权限后重试。",
                &words,
            ),
            words,
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(CallError::internal)
}
