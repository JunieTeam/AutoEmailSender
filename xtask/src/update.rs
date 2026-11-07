//! `cargo xtask publish-update <tag> <directory>` copies a release to the update mirror, from the
//! files of its GitHub Release or of a local build (docs/development.md#发布).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use api::updater::MIRROR;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;

/// The rclone remote and the bucket behind the mirror.
const BUCKET: &str = "aes-dl:aes-dl-1301555531";

pub async fn publish(tag: &str, dir: &Path) -> crate::Result {
    let version = tag
        .strip_prefix('v')
        .filter(|version| !version.is_empty())
        .ok_or("标签写成 v 加版本号，比如 v3.0.1")?;
    let (macos, macos_signature) = package(dir, ".app.tar.gz", version)?;
    let (windows, windows_signature) = package(dir, "-setup.exe", version)?;
    let dmg = only(dir, ".dmg")?;
    let notes_path = dir.join("notes.md");
    let notes = std::fs::read_to_string(&notes_path)
        .map_err(|e| format!("读不到更新说明 {}：{e}", notes_path.display()))?;
    // Each run gets a directory of its own and overwrites nothing: the CDN keeps files for a
    // month, so a client still on the previous manifest would get a package failing its signature.
    let now = chrono::Utc::now();
    let folder = format!("{version}/{}", now.format("%Y%m%dT%H%M%SZ"));
    let key = |file: &Path| format!("{folder}/{}", name(file));
    let manifest = serde_json::to_vec_pretty(&serde_json::json!({
        "version": version,
        "notes": notes.trim(),
        "pub_date": now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "platforms": {
            "darwin-aarch64": { "signature": macos_signature, "url": format!("{MIRROR}{}", key(&macos)) },
            "windows-x86_64": { "signature": windows_signature, "url": format!("{MIRROR}{}", key(&windows)) },
        },
    }))?;
    let net = net::Net::new(Vec::new()).map_err(|e| format!("准备网络连接失败：{e}"))?;
    // Each file is read back through the CDN before the manifest leads clients to it.
    for file in [&macos, &windows, &dmg] {
        let key = key(file);
        rclone(
            Command::new("rclone")
                .arg("copyto")
                .arg(file)
                .arg(format!("{BUCKET}/{key}")),
            None,
        )?;
        if served(&net, &format!("{MIRROR}{key}")).await? != std::fs::read(file)? {
            return Err(format!(
                "从备用地址读回的 {key} 和本地的不一样，没有上传清单；先查 CDN 的回源和缓存配置"
            )
            .into());
        }
    }
    rclone(
        Command::new("rclone")
            .arg("rcat")
            .arg(format!("{BUCKET}/latest.json")),
        Some(&manifest),
    )?;
    if served(&net, &format!("{MIRROR}latest.json")).await? != manifest {
        return Err("从备用地址读回的 latest.json 和刚上传的不一样".into());
    }
    println!("已把 {tag} 发布到 {MIRROR}");
    Ok(())
}

/// The one file in `dir` ending in `suffix`, with its signature, which must be for `version`:
/// the updater refuses a package signed for another version than its manifest gives.
fn package(dir: &Path, suffix: &str, version: &str) -> crate::Result<(PathBuf, String)> {
    let path = only(dir, suffix)?;
    let mut signature_path = path.clone().into_os_string();
    signature_path.push(".sig");
    let signature_path = PathBuf::from(signature_path);
    let signature = std::fs::read_to_string(&signature_path)
        .map_err(|e| format!("读不到签名 {}：{e}", signature_path.display()))?;
    let signature = signature.trim();
    let decoded = STANDARD
        .decode(signature)
        .ok()
        .and_then(|text| String::from_utf8(text).ok());
    let signed = decoded.as_deref().and_then(signed_version);
    if signed != Some(version) {
        return Err(format!(
            "{} 的签名不是 {version} 的（签名里的版本：{}）",
            name(&path),
            signed.unwrap_or("没有")
        )
        .into());
    }
    Ok((path, signature.to_owned()))
}

/// The version in a minisign signature's trusted comment, where the Tauri CLI writes it as one of
/// its tab separated `key:value` fields.
fn signed_version(signature: &str) -> Option<&str> {
    signature
        .lines()
        .find_map(|line| line.strip_prefix("trusted comment: "))?
        .split('\t')
        .find_map(|field| field.strip_prefix("version:"))
}

/// The one file in `dir` whose name ends in `suffix`.
fn only(dir: &Path, suffix: &str) -> crate::Result<PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(suffix))
        {
            found.push(path);
        }
    }
    <[PathBuf; 1]>::try_from(found)
        .map(|[path]| path)
        .map_err(|found| {
            format!(
                "{} 里要有且只有一个以 {suffix} 结尾的文件，现在有 {} 个",
                dir.display(),
                found.len()
            )
            .into()
        })
}

/// The file's name on the mirror, with spaces turned into dots as GitHub does, so that its URL
/// needs no escaping.
fn name(file: &Path) -> String {
    file.file_name()
        .unwrap_or_else(|| unreachable!("only files from the directory listing are named"))
        .to_string_lossy()
        .replace(' ', ".")
}

fn rclone(command: &mut Command, input: Option<&[u8]>) -> crate::Result {
    let mut child = command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .spawn()
        .map_err(|e| {
            format!("运行 rclone 失败：{e}；先装好 rclone 并配好远端，见 docs/development.md#发布")
        })?;
    if let Some(input) = input {
        // Dropping the pipe after writing ends rclone's input.
        child
            .stdin
            .take()
            .unwrap_or_else(|| unreachable!("stdin is piped"))
            .write_all(input)?;
    }
    let status = child.wait()?;
    if !status.success() {
        return Err(format!("rclone 没有完成：{status}").into());
    }
    Ok(())
}

/// What the mirror serves at `url`.
async fn served(net: &net::Net, url: &str) -> crate::Result<Vec<u8>> {
    let response = net.http().get(url).send().await?.error_for_status()?;
    Ok(response.bytes().await?.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Tauri CLI's signature of a 3.0.0-alpha.0 update package, made with a throwaway key.
    const SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVRd1ZFd1FrSmhnZERVWm9aMUczYzNPckx1NEU1bC9QeFM0MXAzQjEyOExOeU4yN0RxNjVJaVVFby9aQWRXaHcvQ25ZN0V4a3hTZk5lZklIekZwUnJva2hJWWYrbWM4UlFzPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzk0MDgzMDgwCWZpbGU6QXV0byBFbWFpbCBTZW5kZXIuYXBwLnRhci5negl2ZXJzaW9uOjMuMC4wLWFscGhhLjAKYml6Z241SlFBY1laYm5XTkVEMi9Lb2pQd2MvYlowdGQ0bFl6cjRvUFN3VzdTTnJkWTEraW8zUDUwd1E2MWJDNTRZZzM0dlVSTFF0TlZHd2pkdzJHQ2c9PQo=";

    #[test]
    fn only_packages_signed_for_the_tag_are_published() -> crate::Result {
        let dir = tempfile::tempdir()?;
        std::fs::write(dir.path().join("Auto Email Sender.app.tar.gz"), "package")?;
        std::fs::write(
            dir.path().join("Auto Email Sender.app.tar.gz.sig"),
            SIGNATURE,
        )?;
        let (path, signature) = package(dir.path(), ".app.tar.gz", "3.0.0-alpha.0")?;
        assert_eq!(name(&path), "Auto.Email.Sender.app.tar.gz");
        assert_eq!(signature, SIGNATURE);
        assert!(package(dir.path(), ".app.tar.gz", "3.0.1").is_err());
        Ok(())
    }
}
