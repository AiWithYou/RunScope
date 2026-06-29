use anyhow::{bail, Context};

use crate::model::ProcessInfo;
use crate::services::process_identity;

pub fn kill_process(process: &ProcessInfo) -> anyhow::Result<()> {
    ensure_not_protected(process)?;
    process_identity::ensure_current_process_matches(process)?;
    kill_pid(process.pid)
}

pub fn kill_tree(targets_root_first: &[ProcessInfo]) -> anyhow::Result<Vec<u32>> {
    if targets_root_first.is_empty() {
        bail!("no process tree targets");
    }
    for process in targets_root_first {
        ensure_not_protected(process)?;
    }

    let mut killed = Vec::new();
    for process in targets_root_first.iter().rev() {
        if let Err(error) = process_identity::ensure_current_process_matches(process) {
            if killed.is_empty() {
                return Err(error);
            }
            bail!(
                "killed {:?}, then PID {} failed identity validation: {error}",
                killed,
                process.pid
            );
        }
        match kill_pid(process.pid) {
            Ok(()) => killed.push(process.pid),
            Err(error) => {
                if killed.is_empty() {
                    return Err(error);
                }
                bail!(
                    "killed {:?}, then failed to kill PID {}: {error}",
                    killed,
                    process.pid
                );
            }
        }
    }
    Ok(killed)
}

pub fn close_process(process: &ProcessInfo) -> anyhow::Result<usize> {
    ensure_not_protected(process)?;
    close_pid(process.pid)
}

fn ensure_not_protected(process: &ProcessInfo) -> anyhow::Result<()> {
    if process.protected {
        if let Some(reason) = &process.protection_reason {
            bail!(
                "PID {} ({}) is protected: {reason}",
                process.pid,
                process.name
            );
        }
        bail!("PID {} ({}) is protected", process.pid, process.name);
    }
    Ok(())
}

#[cfg(windows)]
fn kill_pid(pid: u32) -> anyhow::Result<()> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, false, pid)
            .with_context(|| format!("failed to open PID {pid} for termination"))?;
        let result =
            TerminateProcess(handle, 1).with_context(|| format!("failed to terminate PID {pid}"));
        let _ = CloseHandle(handle);
        result
    }
}

#[cfg(not(windows))]
fn kill_pid(_pid: u32) -> anyhow::Result<()> {
    bail!("process termination is implemented for Windows only")
}

#[cfg(windows)]
fn close_pid(pid: u32) -> anyhow::Result<usize> {
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
    };

    struct CloseSearch {
        pid: u32,
        posted: usize,
    }

    unsafe extern "system" fn enum_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = &mut *(lparam.0 as *mut CloseSearch);
        let mut window_pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut window_pid));
        if window_pid == search.pid
            && IsWindowVisible(hwnd).as_bool()
            && PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)).is_ok()
        {
            search.posted += 1;
        }
        BOOL(1)
    }

    let mut search = CloseSearch { pid, posted: 0 };
    unsafe {
        EnumWindows(
            Some(enum_window),
            LPARAM(&mut search as *mut CloseSearch as isize),
        )
        .with_context(|| format!("failed to enumerate windows for PID {pid}"))?;
    }

    if search.posted == 0 {
        bail!("PID {pid} has no visible top-level window to close");
    }
    Ok(search.posted)
}

#[cfg(not(windows))]
fn close_pid(_pid: u32) -> anyhow::Result<usize> {
    bail!("window close is implemented for Windows only")
}
