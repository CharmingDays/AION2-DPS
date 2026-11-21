//! Idempotent migration off the old per-user install folder.
//!
//! `productName` is now "AION2 DPS", so the NSIS installer puts the exe in
//! `%LOCALAPPDATA%\AION2 DPS`. Sessions, alerts, logs and icon caches stay in
//! `%LOCALAPPDATA%\aion2dps` ([`aion2dps::storage::folder`]) and must not be
//! deleted. The WebView profile lives under the unchanged identifier
//! (`%LOCALAPPDATA%\com.aion2dps.meter`) and is never touched.
//!
//! This runs only when the current exe is the installed copy in the new
//! folder. It re-points the existing "\Aion Tallymark" logon task to this exe,
//! then removes only the old installer files, the old Start menu shortcut and
//! the old uninstall registry key. It never runs `uninstall.exe` (that would
//! wipe the data folder) and it never prompts for elevation.

use std::path::{Path, PathBuf};

const NEW_DIR: &str = "AION2 DPS";
const OLD_DIR: &str = "aion2dps";
const OLD_EXE: &str = "aion2dps.exe";
const OLD_UNINSTALLER: &str = "uninstall.exe";
const OLD_SHORTCUT: &str = "aion2dps.lnk";
const OLD_UNINSTALL_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\aion2dps";

/// If this process is the new installed copy, clean up the previous install.
pub fn run() {
    #[cfg(windows)]
    run_windows();
}

#[cfg(windows)]
fn run_windows() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) else {
        return;
    };
    if !is_new_install(&exe, &local) {
        return;
    }
    tracing::info!(target: "overlay", "migrating from the previous install folder if it is still present");
    crate::startup::repoint_existing(&exe);
    remove_file_if_present(&old_exe(&local));
    remove_file_if_present(&old_uninstaller(&local));
    if let Some(roaming) = std::env::var_os("APPDATA").map(PathBuf::from) {
        remove_file_if_present(&old_start_menu_shortcut(&roaming));
    }
    delete_old_uninstall_key();
}

pub(crate) fn new_install_dir(local_app_data: &Path) -> PathBuf {
    local_app_data.join(NEW_DIR)
}

pub(crate) fn old_exe(local_app_data: &Path) -> PathBuf {
    local_app_data.join(OLD_DIR).join(OLD_EXE)
}

pub(crate) fn old_uninstaller(local_app_data: &Path) -> PathBuf {
    local_app_data.join(OLD_DIR).join(OLD_UNINSTALLER)
}

pub(crate) fn old_start_menu_shortcut(roaming_app_data: &Path) -> PathBuf {
    roaming_app_data
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join(OLD_SHORTCUT)
}

/// True when `exe` is `aion2dps.exe` inside `%LOCALAPPDATA%\AION2 DPS`.
pub(crate) fn is_new_install(exe: &Path, local_app_data: &Path) -> bool {
    let Some(dir) = exe.parent() else {
        return false;
    };
    let Some(name) = exe.file_name() else {
        return false;
    };
    if !name.eq_ignore_ascii_case(OLD_EXE) {
        return false;
    }
    if !dir.join(OLD_UNINSTALLER).is_file() {
        return false;
    }
    paths_eq(dir, &new_install_dir(local_app_data))
}

fn paths_eq(a: &Path, b: &Path) -> bool {
    if let (Ok(a), Ok(b)) = (dunce_or_canon(a), dunce_or_canon(b)) {
        return a.as_os_str().eq_ignore_ascii_case(b.as_os_str());
    }
    a.as_os_str().eq_ignore_ascii_case(b.as_os_str())
}

fn dunce_or_canon(path: &Path) -> std::io::Result<PathBuf> {
    let canon = path.canonicalize()?;
    Ok(PathBuf::from(
        canon
            .to_string_lossy()
            .replacen(r"\\?\", "", 1),
    ))
}

#[cfg(windows)]
fn remove_file_if_present(path: &Path) {
    if !path.is_file() {
        return;
    }
    match std::fs::remove_file(path) {
        Ok(()) => tracing::info!(target: "overlay", "removed {}", path.display()),
        Err(err) => tracing::warn!(target: "overlay", "could not remove {}: {err}", path.display()),
    }
}

#[cfg(windows)]
fn delete_old_uninstall_key() {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    let output = Command::new("reg.exe")
        .args(["delete", OLD_UNINSTALL_KEY, "/f"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x0800_0000)
        .output();
    match output {
        Ok(out) if out.status.success() => {
            tracing::info!(target: "overlay", "removed uninstall key {OLD_UNINSTALL_KEY}");
        }
        Ok(_) => {}
        Err(err) => tracing::warn!(target: "overlay", "could not remove uninstall key {OLD_UNINSTALL_KEY}: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_paths_are_the_previous_install_only() {
        let local = Path::new(r"C:\Users\example\AppData\Local");
        let roaming = Path::new(r"C:\Users\example\AppData\Roaming");
        assert_eq!(
            new_install_dir(local),
            PathBuf::from(r"C:\Users\example\AppData\Local\AION2 DPS")
        );
        assert_eq!(
            old_exe(local),
            PathBuf::from(r"C:\Users\example\AppData\Local\aion2dps\aion2dps.exe")
        );
        assert_eq!(
            old_uninstaller(local),
            PathBuf::from(r"C:\Users\example\AppData\Local\aion2dps\uninstall.exe")
        );
        assert_eq!(
            old_start_menu_shortcut(roaming),
            PathBuf::from(
                r"C:\Users\example\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\aion2dps.lnk"
            )
        );
        assert_ne!(
            old_exe(local).parent().unwrap(),
            new_install_dir(local).as_path()
        );
    }

    #[test]
    fn new_install_requires_the_exact_new_folder_and_exe_name() {
        let local = Path::new(r"C:\Users\example\AppData\Local");
        let new_exe = new_install_dir(local).join("aion2dps.exe");
        // No uninstall.exe beside it in this fake path, so it is not treated
        // as an installed copy (portable exe in a similarly named folder).
        assert!(!is_new_install(&new_exe, local));
        assert!(!is_new_install(
            &old_exe(local),
            local
        ));
        assert!(!is_new_install(
            &local.join("somewhere").join("aion2dps.exe"),
            local
        ));
    }
}
