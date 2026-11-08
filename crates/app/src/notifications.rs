//! The system's permission to show the app's notifications (docs/design.md#桌面应用).
//! `tauri-plugin-notification` shows them, but on the desktop its permission calls always answer
//! granted, so macOS is asked here.

use api::notifications::NotificationPermission;

/// Whether the system shows the app's notifications.
#[tauri::command]
pub async fn notification_permission() -> Result<NotificationPermission, String> {
    system::permission().await
}

/// Asks the system to show the app's notifications; answers once the user has chosen.
#[tauri::command]
pub async fn request_notification_permission() -> Result<NotificationPermission, String> {
    system::request().await
}

/// Opens the app's page in the system's notification settings.
#[cfg(target_os = "macos")]
#[tauri::command]
pub fn open_notification_settings(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let url = format!(
        "x-apple.systempreferences:com.apple.Notifications-Settings.extension?id={}",
        app.config().identifier
    );
    app.opener().open_url(url, None::<&str>).map_err(|e| {
        tracing::warn!("open notification settings: {e}");
        "未能打开系统设置，请在系统设置的“通知”里找到 Auto Email Sender".into()
    })
}

#[cfg(not(target_os = "macos"))]
mod system {
    use api::notifications::NotificationPermission;

    // Windows shows notifications without asking first.
    pub async fn permission() -> Result<NotificationPermission, String> {
        Ok(NotificationPermission::Unsupported)
    }

    pub async fn request() -> Result<NotificationPermission, String> {
        Ok(NotificationPermission::Unsupported)
    }
}

#[cfg(target_os = "macos")]
mod system {
    use std::ptr::NonNull;
    use std::sync::Mutex;

    use api::notifications::NotificationPermission;
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSBundle, NSError};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNAuthorizationStatus, UNNotificationSettings,
        UNUserNotificationCenter,
    };
    use tokio::sync::oneshot;

    const UNREADABLE: &str = "未能读取系统的通知设置，请稍后重试";

    /// The notification center needs the app's bundle. A bare executable, as `pnpm dev` runs, has
    /// none, and asking for the center then raises an exception that ends the process.
    fn center() -> Option<Retained<UNUserNotificationCenter>> {
        NSBundle::mainBundle()
            .bundlePath()
            .to_string()
            .ends_with(".app")
            .then(UNUserNotificationCenter::currentNotificationCenter)
    }

    pub async fn permission() -> Result<NotificationPermission, String> {
        let (sender, receiver) = oneshot::channel();
        // The center and the handler stay on this thread; only the answer is awaited.
        {
            let Some(center) = center() else {
                return Ok(NotificationPermission::Unsupported);
            };
            // Called once, on a thread of the system's.
            let sender = Mutex::new(Some(sender));
            let done = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
                #[expect(unsafe_code, reason = "the system's settings, valid during the call")]
                let status = unsafe { settings.as_ref() }.authorizationStatus();
                let sender = sender
                    .lock()
                    .unwrap_or_else(|_| unreachable!("taking the sender cannot panic"))
                    .take();
                if let Some(sender) = sender {
                    // The receiver is gone only when the window stopped waiting.
                    let _ = sender.send(status);
                }
            });
            center.getNotificationSettingsWithCompletionHandler(&done);
        }
        let status = receiver.await.map_err(|_| UNREADABLE.to_string())?;
        Ok(match status {
            UNAuthorizationStatus::NotDetermined => NotificationPermission::NotAsked,
            UNAuthorizationStatus::Denied => NotificationPermission::Denied,
            // Authorized, or provisionally, quietly in the notification center.
            _ => NotificationPermission::Allowed,
        })
    }

    pub async fn request() -> Result<NotificationPermission, String> {
        let (sender, receiver) = oneshot::channel();
        {
            let Some(center) = center() else {
                return Ok(NotificationPermission::Unsupported);
            };
            let sender = Mutex::new(Some(sender));
            let done = RcBlock::new(move |_granted: Bool, error: *mut NSError| {
                #[expect(unsafe_code, reason = "the system's error, valid during the call")]
                let error = unsafe { error.as_ref() };
                if let Some(error) = error {
                    tracing::warn!(
                        "notification permission request: {} ({} {})",
                        error.localizedDescription(),
                        error.domain(),
                        error.code()
                    );
                }
                let sender = sender
                    .lock()
                    .unwrap_or_else(|_| unreachable!("taking the sender cannot panic"))
                    .take();
                if let Some(sender) = sender {
                    let _ = sender.send(error.is_none());
                }
            });
            // Asks the user only the first time; afterwards answers with the choice made.
            center.requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
                &done,
            );
        }
        let asked = receiver.await.map_err(|_| UNREADABLE.to_string())?;
        if !asked {
            return Err(
                "未能向系统请求通知权限，请在系统设置的“通知”里允许 Auto Email Sender".into(),
            );
        }
        // The answer is only yes or no; the settings tell which of the states the page shows.
        permission().await
    }
}
