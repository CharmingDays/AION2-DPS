//! "Check for updates" in the detailed meter: ask GitHub for the latest
//! release, and on "Update now" download its NSIS installer, check it against
//! the release's SHA256SUMS.txt, start it and quit so no file is locked.
//!
//! When Config allows it (on by default), a background thread asks the same
//! GitHub latest-release URL shortly after start and then every hour. A newer
//! version is announced once (`AVAILABLE`), the meter pill can turn on, and
//! `Live::update_available` holds the version for the UI. Failures stay in
//! app.log. Nothing is downloaded or installed until the user clicks.
//!
//! The only traffic is to GitHub (api.github.com and the release download
//! links), through Windows' own curl.exe like the pet icons. Nothing goes to
//! the game.
//!
//! Not tauri-plugin-updater: that needs a signing key pair, a signed
//! `latest.json` and a changed release process. Here the SHA-256 in
//! SHA256SUMS.txt (from the same release) guards the download instead.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use aion2dps::AppState;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

/// Browser and User-Agent links stay on this name until the repo is renamed:
/// GitHub will redirect it afterwards, and the new name 404s until then.
const REPO: &str = "CharmingDays/aion-tallymark";
/// Try the upcoming name first, then the current one if GitHub answers 404.
/// After the rename, the old API URL is redirected (curl already follows with -L).
const LATEST_URLS: [&str; 2] = [
    "https://api.github.com/repos/CharmingDays/AION2-DPS/releases/latest",
    "https://api.github.com/repos/CharmingDays/aion-tallymark/releases/latest",
];
const SUMS: &str = "SHA256SUMS.txt";
const OWNER: &str = "CharmingDays";
/// Current and upcoming repository names; matched case-insensitively.
const REPOS: [&str; 2] = ["AION2-DPS", "aion-tallymark"];
/// Hosts a release download may end up on after GitHub's redirects.
const DOWNLOAD_HOSTS: [&str; 3] = [
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];
/// A check is reused for this long unless the button asks again.
const CACHE: Duration = Duration::from_secs(30 * 60);
/// First automatic check, after the windows are up.
const START_DELAY: Duration = Duration::from_secs(10);
/// How often automatic checks run after that.
const INTERVAL: Duration = Duration::from_secs(60 * 60);
/// Biggest installer we accept (v0.3.0's is about 4 MB).
const MAX_SETUP: u64 = 200 * 1024 * 1024;
pub const PROGRESS: &str = "update-progress";
/// Fired at most once per new version when an automatic check finds an update.
pub const AVAILABLE: &str = "update-available";

/// What the page shows after a check.
#[derive(Debug, Clone)]
pub struct Check {
    pub current: String,
    pub latest: String,
    pub newer: bool,
    pub name: String,
    pub notes: String,
    pub url: String,
    /// The installer's file name, or "" when the release has none.
    pub asset: String,
    pub size: u64,
}

#[derive(Debug, Clone)]
struct Release {
    check: Check,
    setup_url: String,
    sums_url: Option<String>,
}

#[derive(Default)]
pub struct Updater {
    last: Mutex<Option<(Instant, Release)>>,
    busy: AtomicBool,
}

impl Check {
    pub fn json(&self) -> Value {
        json!({
            "current": self.current,
            "latest": self.latest,
            "newer": self.newer,
            "name": self.name,
            "notes": self.notes,
            "url": self.url,
            "asset": self.asset,
            "size": self.size,
            "installed": installed(),
        })
    }
}

/// `0.3.0`, `v0.3.1`, `1.2.3-beta.1` as comparable numbers; a pre-release
/// sorts before its release.
fn version(text: &str) -> Option<(u64, u64, u64, bool)> {
    let text = text.trim().trim_start_matches(['v', 'V']);
    let (core, pre) = match text.split_once(['-', '+']) {
        Some((core, rest)) => (core, text[core.len()..].starts_with('-') && !rest.is_empty()),
        None => (text, false),
    };
    let mut parts = core.split('.').map(|p| p.parse::<u64>());
    let major = parts.next()?.ok()?;
    let minor = parts.next().unwrap_or(Ok(0)).ok()?;
    let patch = parts.next().unwrap_or(Ok(0)).ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch, !pre))
}

/// True when `latest` is a higher version than `current`.
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (version(latest), version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

fn curl() -> Command {
    let mut cmd = Command::new(if cfg!(windows) { "curl.exe" } else { "curl" });
    cmd.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn agent(current: &str) -> String {
    format!("User-Agent: aion2dps/{current} (+https://github.com/{REPO})")
}

/// GET `url` into memory; the body, the HTTP status and the URL it ended
/// on after redirects.
fn fetch(url: &str, current: &str, limit: u64) -> Result<(Vec<u8>, u32, String), String> {
    let out = curl()
        .args(["-sSL", "--proto", "=https", "--proto-redir", "=https", "--max-redirs", "5"])
        .args(["--max-time", "20", "--max-filesize"])
        .arg(limit.to_string())
        .args(["-H", "Accept: application/vnd.github+json", "-H"])
        .arg(agent(current))
        .args(["-w", "\n%{http_code} %{url_effective}"])
        .arg(url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|err| format!("could not run curl.exe: {err}"))?;
    let mut body = out.stdout;
    let cut = body.iter().rposition(|b| *b == b'\n').unwrap_or(0);
    let tail = String::from_utf8_lossy(&body[cut..]).trim().to_string();
    let (code, effective) = tail.split_once(' ').unwrap_or((tail.as_str(), ""));
    let status = code.parse::<u32>().unwrap_or(0);
    let effective = effective.trim().to_string();
    body.truncate(cut);
    if !out.status.success() || status == 0 {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        tracing::warn!(target: "update", "request to {url} failed: {err}");
        return Err("Could not reach GitHub. Check your internet connection and try again.".into());
    }
    Ok((body, status, effective))
}

/// The host of an `https://` URL, lowercased; `None` for any other scheme
/// or a URL with user info or a port.
fn https_host(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains(['@', ':', '\\']) {
        return None;
    }
    Some(authority.to_ascii_lowercase())
}

/// A release file of this repository on github.com, over https.
/// Accepts both the current name (aion-tallymark) and the upcoming name
/// (AION2-DPS). Owner and repo are matched case-insensitively (GitHub treats
/// them that way); the host and the `/releases/download/` path shape stay exact.
fn is_release_download(url: &str) -> bool {
    if url.chars().any(|c| c.is_whitespace() || c.is_control() || c == '\\') {
        return false;
    }
    if https_host(url).as_deref() != Some("github.com") {
        return false;
    }
    let Some(rest) = url.strip_prefix("https://github.com/") else {
        return false;
    };
    let mut parts = rest.splitn(3, '/');
    let owner = parts.next().unwrap_or("");
    let repo = parts.next().unwrap_or("");
    let tail = parts.next().unwrap_or("");
    if !owner.eq_ignore_ascii_case(OWNER) {
        return false;
    }
    if !REPOS.iter().any(|name| repo.eq_ignore_ascii_case(name)) {
        return false;
    }
    tail.strip_prefix("releases/download/")
        .is_some_and(|file| !file.is_empty())
}

/// Where a release download may land after redirects: GitHub's own hosts.
fn is_allowed_download_host(url: &str) -> bool {
    https_host(url).is_some_and(|host| DOWNLOAD_HOSTS.contains(&host.as_str()))
}

/// Prefer the historical `aion2dps_<ver>_x64-setup.exe` name, then the name
/// Tauri derives from productName ("AION2 DPS"), then any `*_x64-setup.exe`.
fn setup_asset<'a>(assets: &'a [(String, String, u64)], latest: &str) -> Option<&'a (String, String, u64)> {
    let preferred = [
        format!("aion2dps_{latest}_x64-setup.exe"),
        format!("AION2 DPS_{latest}_x64-setup.exe"),
    ];
    for want in &preferred {
        if let Some(asset) = assets.iter().find(|(name, ..)| name == want) {
            return Some(asset);
        }
    }
    assets.iter().find(|(name, ..)| name.ends_with("_x64-setup.exe"))
}

fn latest_release(current: &str) -> Result<(Vec<u8>, u32), String> {
    for (i, url) in LATEST_URLS.iter().enumerate() {
        tracing::info!(target: "update", "checking {url} (current {current})");
        let (body, status, _) = fetch(url, current, 2 * 1024 * 1024)?;
        if status == 404 && i + 1 < LATEST_URLS.len() {
            tracing::info!(
                target: "update",
                "{url} was not found; trying the other repository name"
            );
            continue;
        }
        return Ok((body, status));
    }
    unreachable!("LATEST_URLS is not empty")
}

fn ask_github(current: &str) -> Result<Release, String> {
    let (body, status) = latest_release(current)?;
    match status {
        200 => {}
        403 | 429 => {
            tracing::warn!(target: "update", "GitHub rate limit (HTTP {status})");
            return Err("GitHub is limiting update checks right now. Try again in an hour.".into());
        }
        404 => return Err("No release is published yet.".into()),
        _ => {
            tracing::warn!(target: "update", "GitHub answered HTTP {status}");
            return Err(format!("GitHub answered HTTP {status}. Try again later."));
        }
    }
    let json: Value = serde_json::from_slice(&body).map_err(|err| {
        tracing::warn!(target: "update", "bad release JSON: {err}");
        "GitHub sent an answer the app could not read.".to_string()
    })?;
    let text = |key: &str| json.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    let tag = text("tag_name");
    let latest = tag.trim_start_matches(['v', 'V']).to_string();
    if version(&latest).is_none() {
        return Err(format!("The latest release has an odd tag ({tag})."));
    }
    let assets: Vec<(String, String, u64)> = json
        .get("assets")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    let name = a.get("name")?.as_str()?.to_string();
                    let url = a.get("browser_download_url")?.as_str()?.to_string();
                    Some((name, url, a.get("size").and_then(Value::as_u64).unwrap_or(0)))
                })
                .collect()
        })
        .unwrap_or_default();
    let setup = setup_asset(&assets, &latest);
    let sums_url = assets.iter().find(|(name, ..)| name == SUMS).map(|(_, url, _)| url.clone());
    let release = Release {
        check: Check {
            current: current.to_string(),
            newer: is_newer(&latest, current),
            latest,
            name: text("name"),
            notes: text("body"),
            url: text("html_url"),
            asset: setup.map(|(name, ..)| name.clone()).unwrap_or_default(),
            size: setup.map(|(.., size)| *size).unwrap_or(0),
        },
        setup_url: setup.map(|(_, url, _)| url.clone()).unwrap_or_default(),
        sums_url,
    };
    tracing::info!(target: "update", latest = %release.check.latest, newer = release.check.newer, asset = %release.check.asset, "update check done");
    Ok(release)
}

/// The latest release against this build; `fresh` skips the half-hour cache.
pub fn check(app: &AppHandle, fresh: bool) -> Result<Check, String> {
    let current = app.package_info().version.to_string();
    let updater = app.state::<Updater>();
    if !fresh {
        if let Some((at, release)) = updater.last.lock().unwrap().as_ref() {
            if at.elapsed() < CACHE {
                return Ok(release.check.clone());
            }
        }
    }
    let release = ask_github(&current)?;
    let check = release.check.clone();
    *updater.last.lock().unwrap() = Some((Instant::now(), release));
    Ok(check)
}

/// After a short delay, then every hour: ask GitHub if Config allows it.
/// Failures stay in app.log. A newer version is announced once ([`AVAILABLE`]).
pub fn start_background(app: AppHandle) {
    thread::spawn(move || {
        thread::sleep(START_DELAY);
        loop {
            poll(&app, true);
            thread::sleep(INTERVAL);
        }
    });
}

/// A one-shot check (Config just turned the automatic checks on).
pub fn kick(app: &AppHandle) {
    let app = app.clone();
    thread::spawn(move || poll(&app, true));
}

fn poll(app: &AppHandle, notify: bool) {
    if !app.state::<Arc<AppState>>().check_updates_automatically() {
        return;
    }
    match check(app, false) {
        Ok(c) => remember(app, &c, notify),
        Err(err) => tracing::warn!(target: "update", "automatic check failed: {err}"),
    }
}

/// Keep the live "update available" state, and optionally announce a new version once.
pub fn remember(app: &AppHandle, check: &Check, notify: bool) {
    let state = app.state::<Arc<AppState>>();
    if check.newer {
        state.set_update_found(Some(check.latest.clone()));
    } else {
        state.set_update_found(None);
        return;
    }
    if !notify || state.update_is_skipped(&check.latest) {
        return;
    }
    if state.update_last_notified().as_deref() == Some(check.latest.as_str()) {
        return;
    }
    if let Err(err) = app.emit(AVAILABLE, check_json(check, state.update_is_skipped(&check.latest))) {
        tracing::warn!(target: "update", "could not emit {AVAILABLE}: {err}");
        return;
    }
    tracing::info!(target: "update", latest = %check.latest, "update available");
    state.set_update_last_notified(Some(check.latest.clone()));
}

/// The check as the page sees it, including whether this version was skipped.
pub fn check_json(check: &Check, skipped: bool) -> Value {
    let mut value = check.json();
    value["skipped"] = json!(skipped);
    value["available"] = json!(check.newer && !skipped);
    value
}

fn emit(app: &AppHandle, stage: &'static str, done: u64, total: u64) {
    let _ = app.emit(PROGRESS, json!({ "stage": stage, "done": done, "total": total }));
}

/// The hash SHA256SUMS.txt lists for `asset` (`<hex>  <name>` lines, `*name` too).
pub fn listed_hash(sums: &str, asset: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        (name == asset && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())).then(|| hash.to_ascii_lowercase())
    })
}

fn sha256(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn download(app: &AppHandle, url: &str, to: &Path, total: u64, current: &str) -> Result<(), String> {
    let _ = fs::remove_file(to);
    let mut child = curl()
        .args(["-sSfL", "--proto", "=https", "--proto-redir", "=https", "--max-redirs", "5"])
        .args(["--max-time", "900", "--max-filesize"])
        .arg(MAX_SETUP.to_string())
        .arg("-H")
        .arg(agent(current))
        .args(["-w", "%{url_effective}"])
        .arg("-o")
        .arg(to)
        .arg(url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("could not run curl.exe: {err}"))?;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|err| err.to_string())? {
            break status;
        }
        let done = fs::metadata(to).map(|m| m.len()).unwrap_or(0);
        emit(app, "download", done, total);
        thread::sleep(Duration::from_millis(250));
    };
    if !status.success() {
        let mut err = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let _ = stderr.read_to_string(&mut err);
        }
        tracing::warn!(target: "update", "download of {url} failed: {}", err.trim());
        let _ = fs::remove_file(to);
        return Err("The download failed. Check your connection and try again.".into());
    }
    let mut effective = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut effective);
    }
    if !is_allowed_download_host(effective.trim()) {
        tracing::warn!(target: "update", "download of {url} ended on {}, not a GitHub host", effective.trim());
        let _ = fs::remove_file(to);
        return Err("The installer download was sent somewhere other than GitHub, so it was deleted and not run.".into());
    }
    let done = fs::metadata(to).map(|m| m.len()).unwrap_or(0);
    emit(app, "download", done, total.max(done));
    Ok(())
}

/// True when this exe was put there by the NSIS installer (it leaves
/// `uninstall.exe` beside it); false for the portable exe, which the installer
/// would not replace.
pub fn installed() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("uninstall.exe").is_file()))
        .unwrap_or(false)
}

/// The fixed release page, for the dialog's link. Kept on the current
/// repository name so the link works both before and after the rename
/// (GitHub redirects the old URL).
pub const RELEASES: &str = "https://github.com/CharmingDays/aion-tallymark/releases/latest";

fn update_dir(version: &str) -> PathBuf {
    std::env::temp_dir().join("aion2dps-update").join(version)
}

/// Download, verify and start the installer of the release from the last
/// check (never a URL from the page), then quit the app.
pub fn install(app: &AppHandle, version: &str) -> Result<(), String> {
    let updater = app.state::<Updater>();
    if updater.busy.swap(true, Ordering::SeqCst) {
        return Err("An update is already downloading.".into());
    }
    let result = install_inner(app, version);
    if let Err(err) = &result {
        tracing::warn!(target: "update", "update to {version} stopped: {err}");
    }
    updater.busy.store(false, Ordering::SeqCst);
    result
}

fn install_inner(app: &AppHandle, version: &str) -> Result<(), String> {
    let release = app
        .state::<Updater>()
        .last
        .lock()
        .unwrap()
        .as_ref()
        .map(|(_, release)| release.clone())
        .filter(|release| release.check.latest == version)
        .ok_or("Check for updates again first.")?;
    let check = &release.check;
    if !check.newer {
        return Err("This version is already installed.".into());
    }
    if !installed() {
        return Err("This is the portable exe, which the installer would not replace. Download the new one from the release page.".into());
    }
    if release.setup_url.is_empty() {
        return Err("This release has no Windows installer to download. Get it from the release page.".into());
    }
    if !is_release_download(&release.setup_url) {
        tracing::warn!(target: "update", "installer link {} is not a GitHub release download", release.setup_url);
        return Err("The installer link in this release is not a GitHub release download, so the update was stopped. Get it from the release page.".into());
    }
    let sums_url = release
        .sums_url
        .as_deref()
        .ok_or("This release has no SHA256SUMS.txt, so the installer can't be checked. Get it from the release page.")?;
    if !is_release_download(sums_url) {
        tracing::warn!(target: "update", "SHA256SUMS.txt link {sums_url} is not a GitHub release download");
        return Err("The SHA256SUMS.txt link in this release is not a GitHub release download, so the update was stopped. Get it from the release page.".into());
    }
    let current = check.current.clone();

    emit(app, "sums", 0, 0);
    let (sums, status, landed) = fetch(sums_url, &current, 64 * 1024)?;
    if !is_allowed_download_host(&landed) {
        tracing::warn!(target: "update", "SHA256SUMS.txt from {sums_url} ended on {landed}, not a GitHub host");
        return Err("SHA256SUMS.txt was sent somewhere other than GitHub, so the update was stopped. Get it from the release page.".into());
    }
    if status != 200 {
        return Err(format!("Could not get SHA256SUMS.txt (HTTP {status})."));
    }
    let want = listed_hash(&String::from_utf8_lossy(&sums), &check.asset)
        .ok_or_else(|| format!("SHA256SUMS.txt has no hash for {}, so it won't be installed.", check.asset))?;

    let dir = update_dir(version);
    fs::create_dir_all(&dir).map_err(|err| format!("could not make {}: {err}", dir.display()))?;
    let part = dir.join(format!("{}.part", check.asset));
    let setup = dir.join(&check.asset);
    tracing::info!(target: "update", "downloading {} to {}", release.setup_url, part.display());
    download(app, &release.setup_url, &part, check.size, &current)?;

    emit(app, "verify", 0, 0);
    let got = sha256(&part).map_err(|err| format!("could not read the download: {err}"))?;
    if got != want {
        let _ = fs::remove_file(&part);
        tracing::warn!(target: "update", "SHA-256 mismatch for {}: got {got}, SHA256SUMS.txt says {want}", check.asset);
        return Err("The download does not match its SHA-256 in SHA256SUMS.txt, so it was deleted and not run.".into());
    }
    let _ = fs::remove_file(&setup);
    fs::rename(&part, &setup).map_err(|err| format!("could not keep the download: {err}"))?;
    tracing::info!(target: "update", "SHA-256 ok ({got}); starting {}", setup.display());

    emit(app, "install", 0, 0);
    start_installer(&setup)?;
    tracing::info!(target: "update", "installer started; quitting so it can replace the app");
    let app = app.clone();
    thread::spawn(move || {
        // Let the reply reach the page, then quit so no file stays locked.
        thread::sleep(Duration::from_millis(600));
        app.exit(0);
    });
    Ok(())
}

/// Tauri's NSIS installer: `/P` progress bar only, `/UPDATE` update in place,
/// `/R` start the app again when done (the same flags tauri-plugin-updater uses).
fn start_installer(setup: &Path) -> Result<(), String> {
    const ARGS: [&str; 3] = ["/P", "/UPDATE", "/R"];
    match Command::new(setup).args(ARGS).spawn() {
        Ok(_) => Ok(()),
        // 740: the installer asks for admin rights; let Windows show its prompt.
        Err(err) if err.raw_os_error() == Some(740) => {
            tracing::info!(target: "update", "installer needs elevation; asking Windows");
            let quoted = setup.display().to_string().replace('\'', "''");
            let mut ps = Command::new("powershell.exe");
            ps.args(["-NoProfile", "-NonInteractive", "-Command"])
                .arg(format!("Start-Process -FilePath '{quoted}' -ArgumentList '/P','/UPDATE','/R' -Verb RunAs"));
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                ps.creation_flags(0x0800_0000);
            }
            match ps.status() {
                Ok(status) if status.success() => Ok(()),
                _ => Err("The installer was not allowed to start.".into()),
            }
        }
        Err(err) => Err(format!("could not start the installer: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(is_newer("v0.3.1", "0.3.0"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "1.0.0-beta.1"));
        assert!(!is_newer("v0.3.0", "0.3.0"));
        assert!(!is_newer("0.2.9", "0.3.0"));
        assert!(!is_newer("0.4.0-rc.1", "0.4.0"));
        assert!(!is_newer("nonsense", "0.3.0"));
    }

    #[test]
    fn only_github_release_downloads_are_accepted() {
        for ok in [
            "https://github.com/CharmingDays/aion-tallymark/releases/download/v0.3.12/SHA256SUMS.txt",
            "https://github.com/CharmingDays/AION2-DPS/releases/download/v0.3.14/SHA256SUMS.txt",
            "https://github.com/charmingdays/aion-tallymark/releases/download/v0.3.12/SHA256SUMS.txt",
            "https://github.com/CHARMINGDAYS/aion2-dps/releases/download/v0.3.14/x",
            "https://github.com/CharmingDays/Aion-Tallymark/releases/download/v1/x",
        ] {
            assert!(is_release_download(ok), "{ok}");
        }
        for bad in [
            "http://github.com/CharmingDays/aion-tallymark/releases/download/v0.3.12/SHA256SUMS.txt",
            "https://github.com.evil.example/CharmingDays/aion-tallymark/releases/download/x",
            "https://evil.example/CharmingDays/aion-tallymark/releases/download/x",
            "https://github.com/someone/else/releases/download/v1/SHA256SUMS.txt",
            "https://github.com/CharmingDays/aion-tallymark/releases/download/",
            "https://github.com/CharmingDays/AION2-DPS/releases/download/",
            "https://github.com/CharmingDays/aion-tallymark/releases/download/v1/a b",
            "https://github.com/CharmingDays/aion-tallymark-extra/releases/download/v1/x",
            "https://github.com/CharmingDays/not-this-repo/releases/download/v1/x",
            "https://github.com/CharmingDays/aion-tallymark/releases/downloads/v1/x",
            "https://github.com/CharmingDays/AION2-DPS/archive/refs/tags/v0.3.14.zip",
            "https://GitHub.com/CharmingDays/aion-tallymark/releases/download/v1/x",
            "",
        ] {
            assert!(!is_release_download(bad), "{bad}");
        }
    }

    #[test]
    fn redirects_must_stay_on_github_hosts() {
        assert!(is_allowed_download_host("https://release-assets.githubusercontent.com/github-production-release-asset/1/2?x=y"));
        assert!(is_allowed_download_host("https://objects.githubusercontent.com/a"));
        assert!(is_allowed_download_host("https://github.com/CharmingDays/aion-tallymark/releases/download/v1/x"));
        for bad in [
            "",
            "http://objects.githubusercontent.com/a",
            "https://objects.githubusercontent.com.evil.example/a",
            "https://user@objects.githubusercontent.com/a",
            "https://objects.githubusercontent.com:8443/a",
            "https://evil.example/?https://github.com/",
        ] {
            assert!(!is_allowed_download_host(bad), "{bad}");
        }
    }

    #[test]
    fn prefers_historical_then_product_name_installer() {
        let assets = vec![
            ("README.md".into(), "https://example/README.md".into(), 1u64),
            ("AION2 DPS_0.3.15_x64-setup.exe".into(), "https://github.com/CharmingDays/aion-tallymark/releases/download/v0.3.15/AION2%20DPS_0.3.15_x64-setup.exe".into(), 10),
            ("aion2dps_0.3.15_x64-setup.exe".into(), "https://github.com/CharmingDays/aion-tallymark/releases/download/v0.3.15/aion2dps_0.3.15_x64-setup.exe".into(), 11),
        ];
        let picked = setup_asset(&assets, "0.3.15").expect("an installer");
        assert_eq!(picked.0, "aion2dps_0.3.15_x64-setup.exe");

        let only_new = vec![(
            "AION2 DPS_0.3.15_x64-setup.exe".into(),
            "https://github.com/CharmingDays/AION2-DPS/releases/download/v0.3.15/AION2%20DPS_0.3.15_x64-setup.exe".into(),
            10u64,
        )];
        let picked = setup_asset(&only_new, "0.3.15").expect("product-name installer");
        assert_eq!(picked.0, "AION2 DPS_0.3.15_x64-setup.exe");
    }

    #[test]
    fn finds_listed_hash() {
        let sums = "881f070e32564b943725765e5d7fd28d84813e385c3475e682ec28709150c1d2  aion2dps_0.3.0_x64-setup.exe\n\
                    987bceb63ac3e487e1a91acc78b59bed70872f571bc5ffdddd687813630408ea *aion2dps_0.3.0_x64-portable.exe";
        assert_eq!(
            listed_hash(sums, "aion2dps_0.3.0_x64-setup.exe").as_deref(),
            Some("881f070e32564b943725765e5d7fd28d84813e385c3475e682ec28709150c1d2")
        );
        assert!(listed_hash(sums, "aion2dps_0.3.0_x64-portable.exe").is_some());
        assert!(listed_hash(sums, "other.exe").is_none());
    }
}
