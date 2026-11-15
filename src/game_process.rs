//! Detect whether the AION 2 client is running by listing process names only.
//!
//! Uses a Toolhelp snapshot of image names. It never opens a handle to another
//! process, never reads its memory, and never sends it anything. The name comes
//! from the Steam install path `...\AION2\Aion2\Binaries\Win64\AION2.exe`.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Confirmed executable name of the AION 2 game client.
pub const AION2_EXE: &str = "AION2.exe";

/// How long a process-list result is reused so toast checks do not snapshot
/// the process list on every notification.
const CACHE_TTL: Duration = Duration::from_secs(2);

struct Cache {
    at: Instant,
    running: bool,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

/// True when a process whose image name is [`AION2_EXE`] is running.
pub fn aion2_running() -> bool {
    let running = check();
    remember(running);
    running
}

/// [`aion2_running`], reused for up to two seconds.
pub fn aion2_running_cached() -> bool {
    if let Some(running) = cached() {
        return running;
    }
    aion2_running()
}

/// Event notification toasts (`upcoming`, `imminent`, `live`) when the
/// "only while Aion 2 is running" setting is on and the client is not
/// running. Test toasts (`preview-…`) and other kinds are not suppressed.
pub fn suppress_event_toast(
    only_when_aion2: bool,
    kind: &str,
    key: &str,
    game_running: bool,
) -> bool {
    only_when_aion2
        && matches!(kind, "upcoming" | "imminent" | "live")
        && !key.starts_with("preview-")
        && !game_running
}

fn cached() -> Option<bool> {
    let cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache
        .as_ref()
        .and_then(|c| (c.at.elapsed() < CACHE_TTL).then_some(c.running))
}

fn remember(running: bool) {
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Cache {
        at: Instant::now(),
        running,
    });
}

fn check() -> bool {
    #[cfg(windows)]
    {
        process_named(AION2_EXE)
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
fn process_named(exe: &str) -> bool {
    use std::mem::size_of;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            cntUsage: 0,
            th32ProcessID: 0,
            th32DefaultHeapID: 0,
            th32ModuleID: 0,
            cntThreads: 0,
            th32ParentProcessID: 0,
            pcPriClassBase: 0,
            dwFlags: 0,
            szExeFile: [0; 260],
        };
        let mut found = false;
        if Process32FirstW(snap, &mut entry) != 0 {
            loop {
                let name = String::from_utf16_lossy(
                    &entry.szExeFile[..entry
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.szExeFile.len())],
                );
                if name.eq_ignore_ascii_case(exe) {
                    found = true;
                    break;
                }
                if Process32NextW(snap, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_client_exe_name_is_aion2() {
        assert_eq!(AION2_EXE, "AION2.exe");
        assert!(AION2_EXE.ends_with(".exe"));
    }

    #[test]
    fn event_toasts_are_suppressed_only_when_the_setting_is_on_and_the_game_is_closed() {
        assert!(!suppress_event_toast(false, "upcoming", "rift@1", false));
        assert!(!suppress_event_toast(true, "upcoming", "rift@1", true));
        assert!(suppress_event_toast(true, "upcoming", "rift@1", false));
        assert!(suppress_event_toast(true, "imminent", "rift@1", false));
        assert!(suppress_event_toast(true, "live", "rift@1", false));
        assert!(!suppress_event_toast(
            true,
            "upcoming",
            "preview-upcoming",
            false
        ));
        assert!(!suppress_event_toast(
            true,
            "imminent",
            "preview-imminent",
            false
        ));
        assert!(!suppress_event_toast(true, "live", "preview-live", false));
        assert!(!suppress_event_toast(true, "pet", "pet:1", false));
        assert!(!suppress_event_toast(true, "cube", "cube:1", false));
    }
}
