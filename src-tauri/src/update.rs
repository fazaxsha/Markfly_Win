//! Update service, a Rust port of `electron/update-service.cjs`.
//!
//! The original only shipped macOS installers and hard-failed on other
//! architectures. This port keeps the security posture (GitHub-only URLs,
//! size and checksum verification) but selects a Windows installer
//! (`.msi` preferred, `.exe` fallback) from the GitHub release assets.
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use markfly_core::config::{
    GITHUB_REPO, MAX_PACKAGE_BYTES, UPDATE_CHECK_INTERVAL_MS, UPDATE_DOWNLOAD_HOSTS,
};

pub const EVENT_UPDATE_STATE: &str = "markfly://update-state";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateState {
    pub revision: u64,
    pub status: String,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub release_info_available: bool,
    pub bytes_received: u64,
    pub total_bytes: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
struct ValidatedAsset {
    name: String,
    size: u64,
    digest: Option<String>,
    url: String,
    version: String,
}

pub struct UpdateService {
    current_version: String,
    revision: u64,
    state: UpdateState,
    latest_release_url: Option<String>,
    validated_asset: Option<ValidatedAsset>,
    installer_path: Option<PathBuf>,
    partial_path: Option<PathBuf>,
    partial_dir: Option<PathBuf>,
}

impl UpdateService {
    pub fn new(current_version: &str) -> Self {
        Self {
            current_version: current_version.to_string(),
            revision: 0,
            state: UpdateState {
                revision: 0,
                status: "idle".into(),
                current_version: current_version.to_string(),
                latest_version: None,
                release_info_available: false,
                bytes_received: 0,
                total_bytes: Option::None,
                error: Option::None,
            },
            latest_release_url: Option::None,
            validated_asset: Option::None,
            installer_path: Option::None,
            partial_path: Option::None,
            partial_dir: Option::None,
        }
    }

    pub fn get_state(&self) -> UpdateState {
        self.state.clone()
    }

    fn set_state(&mut self, app: &AppHandle, status: &str) -> UpdateState {
        self.state.status = status.to_string();
        self.revision += 1;
        self.state.revision = self.revision;
        let snapshot = self.state.clone();
        let _ = app.emit(EVENT_UPDATE_STATE, snapshot.clone());
        snapshot
    }

    fn patch(
        &mut self,
        app: &AppHandle,
        status: &str,
        latest_version: Option<String>,
        release_info_available: bool,
        error: Option<String>,
        bytes_received: u64,
        total_bytes: Option<u64>,
    ) -> UpdateState {
        self.state.latest_version = latest_version;
        self.state.release_info_available = release_info_available;
        self.state.error = error;
        self.state.bytes_received = bytes_received;
        self.state.total_bytes = total_bytes;
        self.set_state(app, status)
    }

    /// Fetches the latest GitHub release and validates a Windows installer.
    pub async fn check(&mut self, app: &AppHandle) -> UpdateState {
        self.patch(app, "checking", Option::None, false, Option::None, 0, Option::None);

        let release = match fetch_latest_release().await {
            Ok(release) => release,
            Err(err) => {
                return self.patch(
                    app,
                    "error",
                    Option::None,
                    false,
                    Some(err),
                    0,
                    Option::None,
                )
            }
        };

        let installed = parse_version(&self.current_version);
        let latest = release
            .tag_name
            .as_deref()
            .and_then(parse_version)
            .map(|parts| version_string(&parts))
            .unwrap_or_default();

        let common = (latest.clone(), true, Option::None);
        self.latest_release_url = release.html_url.clone();

        let Some(installed) = installed else {
            return self.patch(
                app,
                "error",
                Option::None,
                false,
                Some("Installed Markfly version is invalid.".into()),
                0,
                Option::None,
            );
        };
        let Some(latest_parts) = release.tag_name.as_deref().and_then(parse_version) else {
            return self.patch(
                app,
                "error",
                Option::None,
                false,
                Some("GitHub release version is invalid.".into()),
                0,
                Option::None,
            );
        };

        if compare_versions(&latest_parts, &installed) <= 0 {
            self.validated_asset = Option::None;
            self.installer_path = Option::None;
            return self.patch(
                app,
                "current",
                Option::None,
                false,
                Option::None,
                0,
                Option::None,
            );
        }

        match select_windows_asset(&release, &latest) {
            Ok(asset) => {
                self.validated_asset = Some(asset);
                self.installer_path = Option::None;
                self.patch(
                    app,
                    "available",
                    Option::from(common.0),
                    common.1,
                    common.2,
                    0,
                    Option::None,
                )
            }
            Err(err) => self.patch(
                app,
                "error",
                Option::from(common.0),
                common.1,
                Some(err),
                0,
                Option::None,
            ),
        }
    }

    /// Checks at most once every 24h (timestamp persisted in the data dir).
    pub async fn check_on_startup(&mut self, app: &AppHandle, data_dir: &std::path::Path) {
        let timestamp_path = data_dir.join("update-check.json");
        let now = now_ms();
        if let Ok(bytes) = std::fs::read(&timestamp_path) {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                if let Some(previous) = value.get("lastCheckedAt").and_then(|v| v.as_u64()) {
                    if now.saturating_sub(previous) < UPDATE_CHECK_INTERVAL_MS {
                        return;
                    }
                }
            }
        }
        let _ = std::fs::create_dir_all(data_dir);
        let _ = std::fs::write(
            timestamp_path,
            serde_json::json!({ "lastCheckedAt": now }).to_string(),
        );
        self.check(app).await;
    }

    /// Downloads and verifies the installer selected by `check`.
    pub async fn download(&mut self, app: &AppHandle) -> UpdateState {
        let Some(asset) = self.validated_asset.clone() else {
            return self.patch(
                app,
                "error",
                Option::None,
                false,
                Some("Check for a compatible update first.".into()),
                0,
                Option::None,
            );
        };
        if self.installer_path.is_some() {
            return self.set_state(app, "ready");
        }

        self.patch(
            app,
            "downloading",
            Option::None,
            false,
            Option::None,
            0,
            Some(asset.size),
        );

        let download_dir = match tempfile_like_dir() {
            Some(dir) => dir,
            Option::None => {
                return self.patch(
                    app,
                    "error",
                    Option::None,
                    false,
                    Some("Could not create a temporary directory.".into()),
                    0,
                    Option::None,
                )
            }
        };
        self.partial_dir = Some(download_dir.clone());
        let destination = download_dir.join(&asset.name);
        self.partial_path = Some(destination.clone());

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10 * 60))
            .build();
        let Ok(client) = client else {
            return self.fail_download(app, "Could not initialize the HTTP client.".into());
        };

        let response = client
            .get(&asset.url)
            .header(reqwest::header::USER_AGENT, "Markfly/0.4")
            .send()
            .await;
        let Ok(response) = response else {
            return self.fail_download(app, "Installer download failed.".into());
        };
        if !response.status().is_success() {
            return self.fail_download(
                app,
                format!("Installer download failed (HTTP {}).", response.status()),
            );
        }

        use futures_util::StreamExt;
        let mut stream = response.bytes_stream();
        let mut file = match tokio::fs::File::create(&destination).await {
            Ok(file) => file,
            Err(err) => return self.fail_download(app, err.to_string()),
        };
        let mut received: u64 = 0;
        while let Some(chunk) = stream.next().await {
            let Ok(chunk) = chunk else {
                return self.fail_download(app, "Installer download failed.".into());
            };
            received += chunk.len() as u64;
            if received > MAX_PACKAGE_BYTES || received > asset.size {
                return self.fail_download(
                    app,
                    "Installer exceeded its expected size.".into(),
                );
            }
            if tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
                .await
                .is_err()
            {
                return self.fail_download(app, "Could not write the installer.".into());
            }
            self.patch(
                app,
                "downloading",
                Option::None,
                false,
                Option::None,
                received,
                Some(asset.size),
            );
        }

        if tokio::io::AsyncWriteExt::flush(&mut file).await.is_err() {
            return self.fail_download(app, "Could not write the installer.".into());
        }
        if received != asset.size {
            return self.fail_download(
                app,
                "Installer size did not match GitHub release metadata.".into(),
            );
        }

        if let Some(expected) = asset.digest.as_deref() {
            let bytes = match tokio::fs::read(&destination).await {
                Ok(bytes) => bytes,
                Err(err) => return self.fail_download(app, err.to_string()),
            };
            use sha2::{Digest, Sha256};
            let digest = format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
            if !digest.eq_ignore_ascii_case(expected) {
                return self.fail_download(
                    app,
                    "Installer checksum did not match GitHub.".into(),
                );
            }
        }

        self.installer_path = Some(destination);
        self.partial_path = Option::None;
        self.partial_dir = Option::None;
        self.set_state(app, "ready")
    }

    fn fail_download(&mut self, app: &AppHandle, error: String) -> UpdateState {
        self.cleanup_partial();
        self.installer_path = Option::None;
        self.patch(app, "error", Option::None, false, Some(error), 0, Option::None)
    }

    fn cleanup_partial(&mut self) {
        if let Some(path) = self.partial_path.take() {
            let _ = std::fs::remove_file(path);
        }
        if let Some(dir) = self.partial_dir.take() {
            let _ = std::fs::remove_dir(dir);
        }
    }

    /// Launches the downloaded installer; the app keeps running so the user
    /// can finish the installation and reopen Markfly.
    pub fn open_installer(&mut self, app: &AppHandle) -> UpdateState {
        let Some(path) = self.installer_path.clone() else {
            return self.patch(
                app,
                "error",
                Option::None,
                false,
                Some("No verified installer is ready to open.".into()),
                0,
                Option::None,
            );
        };
        let is_msi = path
            .extension()
            .map(|ext| ext.eq_ignore_ascii_case("msi"))
            .unwrap_or(false);
        let mut command = if is_msi {
            let mut msiexec = std::process::Command::new("msiexec");
            msiexec.arg("/i").arg(&path);
            msiexec
        } else {
            std::process::Command::new(&path)
        };
        match command.spawn() {
            Ok(_) => self.patch(
                app,
                "opening",
                Option::None,
                false,
                Some("Finish the installation, then reopen Markfly.".into()),
                0,
                Option::None,
            ),
            Err(err) => self.patch(
                app,
                "error",
                Option::None,
                false,
                Some(format!("Could not open the installer: {err}")),
                0,
                Option::None,
            ),
        }
    }

    /// Opens the GitHub release page in the browser.
    pub async fn open_release(&mut self, app: &AppHandle) -> Result<(), String> {
        let Some(url) = self.latest_release_url.clone() else {
            return Err("No validated GitHub release link is available.".into());
        };
        crate::commands::open_external_impl(app, &url)
    }
}

#[derive(Debug, Deserialize)]
struct Release {
    draft: Option<bool>,
    prerelease: Option<bool>,
    tag_name: Option<String>,
    html_url: Option<String>,
    assets: Option<Vec<Asset>>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: Option<String>,
    size: Option<u64>,
    digest: Option<String>,
    browser_download_url: Option<String>,
}

async fn fetch_latest_release() -> Result<Release, String> {
    let api = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(15_000))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|err| err.to_string())?;
    let response = client
        .get(&api)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header(reqwest::header::USER_AGENT, "Markfly-Updater")
        .send()
        .await
        .map_err(|_| "GitHub release check timed out.".to_string())?;

    if response.status().is_redirection() {
        return Err("GitHub release lookup returned an invalid redirect.".into());
    }
    if response.status() == 403 {
        return Err("GitHub rate limit reached. Try again later.".into());
    }
    if !response.status().is_success() {
        return Err(format!(
            "GitHub release check failed (HTTP {}).",
            response.status()
        ));
    }

    let release: Release = response
        .json()
        .await
        .map_err(|_| "GitHub returned invalid release data.".to_string())?;
    if release.draft.unwrap_or(false) || release.prerelease.unwrap_or(false) {
        return Err("GitHub latest release data is invalid.".into());
    }
    Ok(release)
}

/// Selects and validates a Windows installer asset from a GitHub release.
///
/// Preference order: `.msi` mentioning the version, then `.exe` mentioning
/// the version, then any `.msi`/`.exe`. The download URL must live on
/// GitHub and inside the project's release path.
fn select_windows_asset(release: &Release, version: &str) -> Result<ValidatedAsset, String> {
    let tag = release.tag_name.clone().unwrap_or_default();
    let assets = release.assets.as_deref().unwrap_or_default();
    let version = version.to_ascii_lowercase();

    let mut candidates: Vec<&Asset> = assets
        .iter()
        .filter(|asset| {
            let name = asset.name.clone().unwrap_or_default().to_ascii_lowercase();
            name.ends_with(".msi") || name.ends_with(".exe")
        })
        .collect();

    let rank = |asset: &Asset| -> u8 {
        let name = asset.name.clone().unwrap_or_default().to_ascii_lowercase();
        let mentions_version = name.contains(&version);
        match (name.ends_with(".msi"), mentions_version) {
            (true, true) => 0,
            (true, false) => 2,
            (false, true) => 1,
            (false, false) => 3,
        }
    };
    candidates.sort_by_key(|asset| rank(asset));

    let Some(asset) = candidates.first() else {
        return Err("No unique compatible installer in this release.".into());
    };
    let name = asset
        .name
        .clone()
        .ok_or("GitHub release asset has no name.")?;
    let size = asset.size.ok_or("GitHub installer size is invalid.")?;
    if size == 0 || size > MAX_PACKAGE_BYTES {
        return Err("GitHub installer size is invalid or exceeds the 250 MiB limit.".into());
    }
    if asset
        .digest
        .as_deref()
        .is_some_and(|digest| !is_sha256_digest(digest))
    {
        return Err("GitHub installer checksum format is invalid.".into());
    }
    let raw_url = asset
        .browser_download_url
        .clone()
        .ok_or("GitHub release asset has no download URL.")?;
    let url = url::Url::parse(&raw_url).map_err(|_| "Release contains an invalid URL.".to_string())?;
    if !UPDATE_DOWNLOAD_HOSTS.contains(&url.host_str().unwrap_or_default()) {
        return Err("Release contains an unsafe URL.".into());
    }
    let expected_prefix = format!("https://github.com/{GITHUB_REPO}/releases/download/{tag}/");
    if !raw_url.starts_with(&expected_prefix) {
        return Err("Installer URL did not match the selected release.".into());
    }

    Ok(ValidatedAsset {
        name,
        size,
        digest: asset.digest.clone(),
        url: raw_url,
        version,
    })
}

fn is_sha256_digest(digest: &str) -> bool {
    let Some(hex_part) = digest.strip_prefix("sha256:") else {
        return false;
    };
    hex_part.len() == 64
        && hex_part
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn version_string(parts: &[u64; 3]) -> String {
    format!("{}.{}.{}", parts[0], parts[1], parts[2])
}

fn parse_version(value: &str) -> Option<[u64; 3]> {
    let trimmed = value.strip_prefix('v').unwrap_or(value);
    let mut parts = trimmed.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return Option::None;
    }
    Some([major, minor, patch])
}

fn compare_versions(left: &[u64; 3], right: &[u64; 3]) -> i32 {
    for index in 0..3 {
        if left[index] != right[index] {
            return if left[index] > right[index] { 1 } else { -1 };
        }
    }
    0
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn tempfile_like_dir() -> Option<PathBuf> {
    let base = std::env::temp_dir().join(format!("markfly-update-{}", std::process::id()));
    std::fs::create_dir_all(&base).ok()?;
    Some(base)
}
