use std::error::Error;
use std::path::{Path, PathBuf};

use api::CallError;
use failure::Failure;
use tauri::AppHandle;
#[cfg(target_os = "macos")]
use tauri::Manager;
use tauri_plugin_autostart::ManagerExt;

pub const ARG: &str = "--autostart";
type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[tauri::command]
pub async fn autostart_enabled(app: AppHandle) -> std::result::Result<bool, CallError> {
    let enabled = app.autolaunch().is_enabled().map_err(|e| {
        CallError::external(
            Failure::unrecognized("无法读取开机自启状态，请稍后重试", &e.to_string()),
            e,
        )
    });
    crate::logged("autostart_enabled", enabled)
}

#[tauri::command]
pub async fn set_autostart(app: AppHandle, enabled: bool) -> std::result::Result<(), CallError> {
    crate::logged("set_autostart", set(&app, enabled))
}

fn set(app: &AppHandle, enabled: bool) -> std::result::Result<(), CallError> {
    if enabled {
        let executable = current_executable().map_err(setting_error)?;
        if translocated(&executable) {
            return Err(CallError::invalid(
                "请先把 Auto Email Sender 拖到“应用程序”文件夹，再从那里打开后开启".into(),
            ));
        }
        enable(app, &executable).map_err(setting_error)
    } else {
        app.autolaunch().disable().map_err(setting_error)?;
        // On Windows, an app run as administrator turns it on for every user of the computer,
        // which only an administrator can turn off again.
        if app.autolaunch().is_enabled().map_err(setting_error)? {
            return Err(CallError::external(
                Failure::new(
                    "开机自启是以管理员身份开启的。右键 Auto Email Sender 选“以管理员身份运行”，再关掉它。",
                ),
                "autostart stays on for every user",
            ));
        }
        Ok(())
    }
}

fn setting_error(e: impl std::fmt::Display) -> CallError {
    CallError::external(
        Failure::unrecognized(
            "未能更改开机自启设置，请检查系统登录项设置后重试",
            &e.to_string(),
        ),
        e,
    )
}

/// Refresh moved applications without rewriting an unchanged macOS login item.
pub fn refresh(app: &AppHandle) -> Result<()> {
    if app.autolaunch().is_enabled()? {
        let executable = current_executable()?;
        if translocated(&executable) {
            return Err("the app runs from a translocated copy".into());
        }
        enable(app, &executable)?;
    }
    Ok(())
}

fn enable(app: &AppHandle, executable: &Path) -> Result<()> {
    if !app.autolaunch().is_enabled()? || !registration_matches(app, executable)? {
        app.autolaunch().enable()?;
    }
    Ok(())
}

fn current_executable() -> std::io::Result<PathBuf> {
    let path = std::env::current_exe()?;
    #[cfg(target_os = "macos")]
    let path = path.canonicalize()?;
    Ok(path)
}

/// Whether macOS runs the app from a temporary copy, which a login item must not point to.
fn translocated(executable: &Path) -> bool {
    cfg!(target_os = "macos") && executable.to_string_lossy().contains("/AppTranslocation/")
}

#[cfg(target_os = "macos")]
fn registration_matches(app: &AppHandle, executable: &Path) -> Result<bool> {
    let path = app
        .path()
        .home_dir()?
        .join("Library/LaunchAgents")
        .join(format!("{}.plist", app.config().identifier));
    let value = plist::Value::from_file(path)?;
    launch_agent_matches(&value, executable)
}

#[cfg(target_os = "macos")]
fn launch_agent_matches(value: &plist::Value, executable: &Path) -> Result<bool> {
    let args = value
        .as_dictionary()
        .and_then(|dict| dict.get("ProgramArguments"))
        .and_then(plist::Value::as_array)
        .ok_or("login item has no ProgramArguments array")?;
    let args = args
        .iter()
        .map(|arg| arg.as_string().ok_or("login item argument is not a string"))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(args == [executable.to_string_lossy().as_ref(), ARG])
}

#[cfg(target_os = "windows")]
fn registration_matches(app: &AppHandle, executable: &Path) -> Result<bool> {
    use windows_registry::{CURRENT_USER, LOCAL_MACHINE};

    // The plugin uses Dynamic mode, so either hive can contain its Run value.
    let expected = format!("{} {ARG}", executable.display());
    let mut found = false;
    for root in [LOCAL_MACHINE, CURRENT_USER] {
        match root
            .open(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run")
            .and_then(|key| key.get_string(&app.config().identifier))
        {
            Ok(command) => {
                if command != expected {
                    return Ok(false);
                }
                found = true;
            }
            // HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND): this hive has no such value.
            Err(e) if e.code().0 == 0x8007_0002_u32.cast_signed() => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(found)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn translocated_app_cannot_enable_autostart() {
        assert!(translocated(Path::new(
            "/private/var/folders/x/AppTranslocation/id/d/\
            Auto Email Sender.app/Contents/MacOS/app"
        )));
        assert!(!translocated(Path::new(
            "/Applications/Auto Email Sender.app/Contents/MacOS/app"
        )));
    }

    #[test]
    fn only_an_unchanged_launch_command_can_skip_registration() -> Result<()> {
        let current = Path::new("/Applications/Auto Email Sender.app/Contents/MacOS/app");
        for (args, matches) in [
            (vec![current.to_string_lossy().as_ref(), ARG], true),
            (
                vec![
                    "/Users/student/Downloads/Auto Email Sender.app/Contents/MacOS/app",
                    ARG,
                ],
                false,
            ),
            (vec![current.to_string_lossy().as_ref()], false),
        ] {
            let mut dict = plist::Dictionary::new();
            dict.insert(
                "ProgramArguments".into(),
                plist::Value::Array(args.into_iter().map(Into::into).collect()),
            );
            assert_eq!(
                launch_agent_matches(&plist::Value::Dictionary(dict), current)?,
                matches
            );
        }
        assert!(
            launch_agent_matches(&plist::Value::Dictionary(plist::Dictionary::new()), current)
                .is_err()
        );
        Ok(())
    }
}
