//! `jira upgrade`: self-update from GitHub Releases.
//!
//! Assets are named `jira-<version>-<target>[.exe]`, each with a
//! `<name>.sha256` checksum (sha256sum format) published next to it.
//!
//! Network split, tuned for mainland-China users:
//! - Version lookup goes to github.com / api.github.com directly. Accelerator
//!   mirrors reliably proxy `/releases/download/<tag>/<asset>` but not
//!   necessarily the `/releases/latest` redirect.
//! - Asset download honors `JIRA_DOWNLOAD_BASE` (default `https://github.com`),
//!   the accelerator prefix in front of `https://github.com`.
//! - Both honor `JIRA_UPGRADE_PROXY` / `HTTPS_PROXY`. The proxy is wired up
//!   explicitly here because reqwest is built without its system-proxy
//!   feature; Jira API traffic stays proxy-free.

use std::{
    cmp::Ordering,
    io::IsTerminal,
    path::{Path, PathBuf},
    time::Duration,
};

use serde_json::json;
use sha2::{Digest, Sha256};

use crate::output;

const REPO: &str = "wuyan19/jira-rs";
const CURRENT: &str = env!("CARGO_PKG_VERSION");
const GITHUB: &str = "https://github.com";
const NETWORK_HINT: &str = "可设置 HTTPS_PROXY 走代理，或把 JIRA_DOWNLOAD_BASE 设为 GitHub 加速站前缀（形如 https://<加速站>/https://github.com）后重试";

/// Release target this binary was built for; "" = no self-update support.
const TARGET: &str = if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
    "x86_64-pc-windows-msvc"
} else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    "x86_64-unknown-linux-musl"
} else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
    "aarch64-unknown-linux-musl"
} else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
    "x86_64-apple-darwin"
} else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    "aarch64-apple-darwin"
} else {
    ""
};
const EXE_SUFFIX: &str = if cfg!(windows) { ".exe" } else { "" };

/// Run `jira upgrade`. `check` only reports; `force` reinstalls even when
/// the current version is not older.
pub async fn run(check: bool, force: bool) {
    if let Err(msg) = run_inner(check, force).await {
        output::error(&format!("upgrade error: {msg}"));
        std::process::exit(1);
    }
}

async fn run_inner(check: bool, force: bool) -> Result<(), String> {
    if TARGET.is_empty() {
        return Err(format!(
            "当前平台不支持自动升级（{}-{}），请手工下载：{GITHUB}/{REPO}/releases/latest",
            std::env::consts::OS,
            std::env::consts::ARCH
        ));
    }

    // One retry each: on a flaky link a single dropped connection should
    // not fail the whole upgrade. 4xx-style answers are not retried.
    let tag = match latest_tag().await {
        Ok(tag) => tag,
        Err(first) => {
            output::error(&format!("查询最新版本失败，1 秒后重试: {first}"));
            tokio::time::sleep(Duration::from_secs(1)).await;
            latest_tag().await?
        }
    };
    let latest = version_of_tag(&tag).to_string();
    let asset = format!("jira-{latest}-{TARGET}{EXE_SUFFIX}");

    let status = match cmp_version(CURRENT, &latest) {
        Ordering::Less => "update_available",
        Ordering::Equal => "up_to_date",
        Ordering::Greater => "newer_than_release",
    };
    if status != "update_available" && !force {
        print_json(json!({ "current": CURRENT, "latest": latest, "status": status }));
        return Ok(());
    }
    if check {
        print_json(json!({
            "current": CURRENT,
            "latest": latest,
            "status": "update_available",
            "asset": asset,
            "url": format!("{GITHUB}/{REPO}/releases/tag/{tag}"),
        }));
        return Ok(());
    }

    let base = download_base();
    let root = format!("{base}/{REPO}/releases/download/{tag}");
    let work = work_dir()?;
    let installed = install(&root, &asset, &work).await;
    let _ = std::fs::remove_dir_all(&work);
    let installed = installed?;

    print_json(json!({
        "upgraded": true,
        "from": CURRENT,
        "to": latest,
        "installedTo": pretty(&installed),
    }));
    output::error(&format!("已升级到 {latest}。正在运行的 `jira mcp` 进程仍是旧版本，重启 MCP 客户端后生效。"));
    Ok(())
}

// ---------------------------------------------------------------------------
// Version lookup and comparison
// ---------------------------------------------------------------------------

/// Latest release tag, or an error carrying the reason both lookups failed.
async fn latest_tag() -> Result<String, String> {
    // 1. Redirect target of .../releases/latest: one request, no API rate limit.
    let probe = client_builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;
    let first = match probe.get(format!("{GITHUB}/{REPO}/releases/latest")).send().await {
        Ok(resp) => {
            let tag = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|loc| loc.trim_end_matches('/').rsplit('/').next())
                .filter(|t| t.starts_with('v'))
                .map(str::to_string);
            match tag {
                Some(tag) => return Ok(tag),
                None => "响应中没有指向 release tag 的 Location".to_string(),
            }
        }
        Err(e) => e.to_string(),
    };

    // 2. REST API (rate-limited per IP, hence second).
    let client = client_builder().build().map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;
    let second = match client
        .get(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
    {
        Ok(resp) => match resp.error_for_status() {
            Ok(resp) => match resp.json::<serde_json::Value>().await {
                Ok(v) => match v["tag_name"].as_str().filter(|t| t.starts_with('v')) {
                    Some(tag) => return Ok(tag.to_string()),
                    None => "返回的最新 release 没有以 v 开头的 tag".to_string(),
                },
                Err(e) => format!("解析响应失败: {e}"),
            },
            Err(e) => e.to_string(),
        },
        Err(e) => e.to_string(),
    };

    Err(format!("无法查询最新版本（github.com: {first}；api.github.com: {second}）。{NETWORK_HINT}"))
}

fn split_version(v: &str) -> (&str, Option<&str>) {
    match v.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (v, None),
    }
}

/// Compare dotted numeric versions; a pre-release sorts before its release.
fn cmp_version(a: &str, b: &str) -> Ordering {
    let (a_core, a_pre) = split_version(a);
    let (b_core, b_pre) = split_version(b);
    let nums = |s: &str| s.split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    let (an, bn) = (nums(a_core), nums(b_core));
    an.iter()
        .cmp(bn.iter())
        .then_with(|| match (a_pre, b_pre) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(x), Some(y)) => x.cmp(y),
        })
}

fn version_of_tag(tag: &str) -> &str {
    tag.strip_prefix('v').unwrap_or(tag)
}

fn download_base() -> String {
    std::env::var("JIRA_DOWNLOAD_BASE")
        .ok()
        .map(|b| b.trim().trim_end_matches('/').to_string())
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| GITHUB.to_string())
}

// ---------------------------------------------------------------------------
// Download and verify
// ---------------------------------------------------------------------------

async fn install(root: &str, asset: &str, work: &Path) -> Result<PathBuf, String> {
    let staged = work.join(asset);
    let url = format!("{root}/{asset}");
    output::error(&format!("下载 {url}"));
    let got = download(&url, &staged).await?;
    let want = checksum(&format!("{url}.sha256")).await?;
    if got != want {
        return Err(format!(
            "SHA-256 校验失败（下载内容 {got}，期望 {want}），未做任何替换。文件可能被截断或篡改，请更换 JIRA_DOWNLOAD_BASE 或改走代理后重试"
        ));
    }

    let exe = std::env::current_exe().map_err(|e| format!("无法定位当前程序路径: {e}"))?;
    // Resolve symlinks so a linked install updates the real file.
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    replace(&staged, &exe)
}

/// Why a download failed. `Retryable` is a broken transfer (connection
/// reset, timeout, premature EOF); `Fatal` is anything retrying cannot fix
/// (an error status from the server, a local write error).
enum DownloadError {
    Retryable(String),
    Fatal(String),
}

impl DownloadError {
    fn message(self) -> String {
        match self {
            Self::Retryable(message) | Self::Fatal(message) => message,
        }
    }
}

/// Download `url` into `dest`, returning the SHA-256 of the bytes written.
/// A broken transfer gets one retry. Progress goes to stderr, and only when
/// it is a terminal (stdout stays JSON).
async fn download(url: &str, dest: &Path) -> Result<String, String> {
    let mut retried = false;
    loop {
        match download_once(url, dest).await {
            Ok(hash) => return Ok(hash),
            Err(DownloadError::Fatal(message)) => return Err(message),
            Err(DownloadError::Retryable(message)) if !retried => {
                retried = true;
                output::error(&format!("\n下载中断，1 秒后重试: {message}"));
                let _ = tokio::fs::remove_file(dest).await;
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(DownloadError::Retryable(message)) => return Err(message),
        }
    }
}

async fn download_once(url: &str, dest: &Path) -> Result<String, DownloadError> {
    let mut resp = fetch(url).await?;
    let total = resp.content_length();
    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| DownloadError::Fatal(format!("创建 {} 失败: {e}", dest.display())))?;
    let (mut hasher, mut done) = (Sha256::new(), 0u64);
    let progress = std::io::stderr().is_terminal();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| DownloadError::Retryable(format!("下载 {url} 中断: {e}")))?
    {
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .map_err(|e| DownloadError::Fatal(format!("写入 {} 失败: {e}", dest.display())))?;
        hasher.update(&chunk);
        done += chunk.len() as u64;
        if progress {
            output::progress(&format!("\r  {done} / {} bytes", total.map_or_else(|| "?".into(), |t| t.to_string())));
        }
    }
    tokio::io::AsyncWriteExt::flush(&mut file)
        .await
        .map_err(|e| DownloadError::Fatal(format!("写入 {} 失败: {e}", dest.display())))?;
    if progress {
        output::progress("\n");
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Expected hash from a `<asset>.sha256` file (sha256sum format).
async fn checksum(url: &str) -> Result<String, String> {
    let text = fetch(url)
        .await
        .map_err(DownloadError::message)?
        .text()
        .await
        .map_err(|e| format!("读取 {url} 失败: {e}"))?;
    text.split_whitespace()
        .next()
        .map(str::to_ascii_lowercase)
        .filter(|h| h.len() == 64)
        .ok_or_else(|| format!("校验文件 {url} 内容无法识别"))
}

async fn fetch(url: &str) -> Result<reqwest::Response, DownloadError> {
    let client = client_builder()
        .build()
        .map_err(|e| DownloadError::Fatal(format!("创建 HTTP 客户端失败: {e}")))?;
    let resp = client.get(url).send().await.map_err(|e| {
        DownloadError::Retryable(format!("请求 {url} 失败: {e}。{NETWORK_HINT}"))
    })?;
    resp.error_for_status()
        .map_err(|e| DownloadError::Fatal(format!("请求 {url} 失败: {e}")))
}


fn client_builder() -> reqwest::ClientBuilder {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(600))
        .user_agent(concat!("jira/", env!("CARGO_PKG_VERSION")));
    if let Some(proxy) = proxy_from_env() {
        builder = builder.proxy(proxy);
    }
    builder
}

fn proxy_from_env() -> Option<reqwest::Proxy> {
    ["JIRA_UPGRADE_PROXY", "HTTPS_PROXY", "https_proxy"]
        .iter()
        .filter_map(|key| std::env::var(key).ok())
        .find(|value| !value.trim().is_empty())
        .and_then(|value| reqwest::Proxy::all(value.trim()).ok())
}

fn work_dir() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("jira-upgrade-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建临时目录 {} 失败: {e}", dir.display()))?;
    Ok(dir)
}

// ---------------------------------------------------------------------------
// Self-replacement
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn replace(staged: &Path, exe: &Path) -> Result<PathBuf, String> {
    // A running .exe can be renamed but neither overwritten nor deleted, so
    // move the old image aside and restore it if writing the new one fails.
    // First drop leftovers of earlier upgrades; those still mapped by a
    // running process cannot go yet and get a process-specific name below.
    let name = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if let Some(dir) = exe.parent() {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            if entry.file_name().to_string_lossy().starts_with(&format!("{name}.old")) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let mut backup = with_suffix(exe, ".old");
    if backup.exists() {
        backup = with_suffix(exe, &format!(".old-{}", std::process::id()));
    }
    std::fs::rename(exe, &backup)
        .map_err(|e| format!("无法替换 {}（目录是否只读？）: {e}", pretty(exe)))?;
    if let Err(e) = std::fs::copy(staged, exe) {
        let _ = std::fs::rename(&backup, exe);
        return Err(format!("写入新版本失败，已恢复原版本: {e}"));
    }
    // Fails harmlessly while an older process still maps the backup.
    let _ = std::fs::remove_file(&backup);
    Ok(exe.to_path_buf())
}

#[cfg(unix)]
fn replace(staged: &Path, exe: &Path) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;

    // Stage in the same directory so the final rename is atomic and the
    // running image keeps executing from the old inode.
    std::fs::set_permissions(staged, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("设置权限失败: {e}"))?;
    let tmp = with_suffix(exe, &format!(".new-{}", std::process::id()));
    let result = std::fs::copy(staged, &tmp).and_then(|_| std::fs::rename(&tmp, exe));
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("无法替换 {}（目录是否需要管理员权限？）: {e}", pretty(exe)));
    }
    Ok(exe.to_path_buf())
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(suffix);
    path.with_file_name(name)
}

/// Path for display: drop the `\\?\` verbatim prefix `canonicalize` adds on
/// Windows (`\\?\UNC\server\share` becomes `\\server\share`).
fn pretty(path: &Path) -> String {
    let s = path.display().to_string();
    match s.strip_prefix(r"\\?\") {
        Some(rest) => match rest.strip_prefix("UNC\\") {
            Some(share) => format!(r"\\{share}"),
            None => rest.to_string(),
        },
        None => s,
    }
}

fn print_json(value: serde_json::Value) {
    output::json(&value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_ordering() {
        assert_eq!(cmp_version("0.1.0", "0.1.1"), Ordering::Less);
        assert_eq!(cmp_version("0.1.1", "0.1.1"), Ordering::Equal);
        assert_eq!(cmp_version("0.1.10", "0.1.9"), Ordering::Greater);
        assert_eq!(cmp_version("0.2", "0.1.9"), Ordering::Greater);
        assert_eq!(cmp_version("1.0.0", "0.9.9"), Ordering::Greater);
    }

    #[test]
    fn prerelease_sorts_before_release() {
        assert_eq!(cmp_version("0.2.0-dev", "0.2.0"), Ordering::Less);
        assert_eq!(cmp_version("0.2.0", "0.2.0-dev"), Ordering::Greater);
        assert_eq!(cmp_version("0.2.0-dev", "0.1.9"), Ordering::Greater);
    }

    #[test]
    fn tag_without_v_prefix_is_tolerated() {
        assert_eq!(version_of_tag("v0.1.1"), "0.1.1");
        assert_eq!(version_of_tag("0.1.1"), "0.1.1");
    }

    #[test]
    fn verbatim_prefix_is_stripped_for_display() {
        assert_eq!(pretty(Path::new(r"\\?\C:\Tools\jira.exe")), r"C:\Tools\jira.exe");
        assert_eq!(pretty(Path::new(r"\\?\UNC\server\share\jira.exe")), r"\\server\share\jira.exe");
        assert_eq!(pretty(Path::new("/usr/local/bin/jira")), "/usr/local/bin/jira");
    }

    #[test]
    fn suffix_appends_after_full_name() {
        assert_eq!(with_suffix(Path::new(r"C:\b\jira.exe"), ".old"), PathBuf::from(r"C:\b\jira.exe.old"));
        assert_eq!(with_suffix(Path::new("/b/jira"), ".new-1"), PathBuf::from("/b/jira.new-1"));
    }
}
