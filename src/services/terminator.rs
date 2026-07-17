use anyhow::{bail, Context};

use crate::model::ProcessInfo;
use crate::services::process_identity;

pub fn kill_process(process: &ProcessInfo) -> anyhow::Result<()> {
    ensure_not_protected(process)?;
    kill_pid(process)
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
        match kill_pid(process) {
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
    close_pid(process)
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
fn kill_pid(process: &ProcessInfo) -> anyhow::Result<()> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    };

    let pid = process.pid;

    unsafe {
        let handle = OpenProcess(
            PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            pid,
        )
        .with_context(|| format!("failed to open PID {pid} for termination"))?;
        let result = (|| {
            validate_process_handle(process, handle)?;
            TerminateProcess(handle, 1).with_context(|| format!("failed to terminate PID {pid}"))
        })();
        let _ = CloseHandle(handle);
        result
    }
}

#[cfg(not(windows))]
fn kill_pid(_process: &ProcessInfo) -> anyhow::Result<()> {
    bail!("process termination is implemented for Windows only")
}

#[cfg(windows)]
fn validate_process_handle(
    process: &ProcessInfo,
    handle: windows::Win32::Foundation::HANDLE,
) -> anyhow::Result<()> {
    use windows::Win32::Foundation::BOOL;
    use windows::Win32::System::Threading::{IsProcessCritical, QueryFullProcessImageNameW};

    let current_start_time = process_identity::process_start_time_from_handle(handle, process.pid)?;
    let expected_start_time = process
        .start_time
        .with_context(|| format!("PID {} has no trusted start time", process.pid))?;
    if current_start_time != expected_start_time {
        bail!(
            "PID {} may now be a different process. Reload and try again.",
            process.pid
        );
    }

    let mut path_buffer = vec![0_u16; 32_768];
    let mut path_length = path_buffer.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            handle,
            Default::default(),
            windows::core::PWSTR(path_buffer.as_mut_ptr()),
            &mut path_length,
        )
        .with_context(|| format!("failed to read PID {} executable path", process.pid))?;
    }
    let path_length = usize::try_from(path_length).context("invalid executable path length")?;
    let current_path = path_buffer
        .get(..path_length)
        .context("executable path length exceeded its buffer")?;
    validate_executable_path(process, &String::from_utf16_lossy(current_path))?;

    let mut critical = BOOL::default();
    unsafe {
        IsProcessCritical(handle, &mut critical)
            .with_context(|| format!("failed to query whether PID {} is critical", process.pid))?;
    }
    if critical.as_bool() {
        bail!(
            "PID {} ({}) is marked critical by Windows and cannot be closed or killed",
            process.pid,
            process.name
        );
    }
    Ok(())
}

#[cfg(windows)]
fn validate_executable_path(process: &ProcessInfo, current_path: &str) -> anyhow::Result<()> {
    let matches = if let Some(expected) = process.exe_path.as_deref() {
        normalize_windows_path(expected).eq_ignore_ascii_case(normalize_windows_path(current_path))
    } else {
        std::path::Path::new(current_path)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(&process.name))
    };
    if matches {
        Ok(())
    } else {
        bail!(
            "PID {} executable no longer matches '{}' (current path '{}'). Reload and try again.",
            process.pid,
            process.exe_path.as_deref().unwrap_or(&process.name),
            current_path
        )
    }
}

#[cfg(windows)]
fn normalize_windows_path(path: &str) -> &str {
    path.strip_prefix(r"\\?\").unwrap_or(path)
}

#[cfg(windows)]
fn close_pid(process: &ProcessInfo) -> anyhow::Result<usize> {
    use windows::Win32::Foundation::{CloseHandle, BOOL, HWND, LPARAM, WPARAM};
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
    };

    let pid = process.pid;

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

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
            .with_context(|| format!("failed to open PID {pid} for window close"))?;
        let result = (|| {
            validate_process_handle(process, handle)?;
            let mut search = CloseSearch { pid, posted: 0 };
            EnumWindows(
                Some(enum_window),
                LPARAM(&mut search as *mut CloseSearch as isize),
            )
            .with_context(|| format!("failed to enumerate windows for PID {pid}"))?;

            if search.posted == 0 {
                bail!("PID {pid} has no visible top-level window to close");
            }
            Ok(search.posted)
        })();
        let _ = CloseHandle(handle);
        result
    }
}

#[cfg(not(windows))]
fn close_pid(_process: &ProcessInfo) -> anyhow::Result<usize> {
    bail!("window close is implemented for Windows only")
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn validates_identity_from_an_open_process_handle() {
        use super::validate_process_handle;
        use crate::collectors::process_collector;
        use crate::settings::Settings;
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

        let processes = process_collector::collect_processes_for_action(&Settings::default())
            .expect("collect current process identity");
        let current = processes
            .iter()
            .find(|process| process.pid == std::process::id())
            .expect("current process is present");

        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, current.pid)
                .expect("open current process");
            validate_process_handle(current, handle).expect("process identity matches");
            let _ = CloseHandle(handle);
        }
    }
}
