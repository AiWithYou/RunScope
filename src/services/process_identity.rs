use std::time::SystemTime;
#[cfg(any(windows, test))]
use std::time::{Duration, UNIX_EPOCH};

use anyhow::anyhow;
#[cfg(windows)]
use anyhow::Context;

use crate::model::ProcessInfo;

pub fn ensure_same_process(expected: &ProcessInfo, current: &ProcessInfo) -> anyhow::Result<()> {
    if !same_process(expected, current) {
        return Err(pid_reused_error(expected.pid));
    }
    Ok(())
}

pub fn same_process(left: &ProcessInfo, right: &ProcessInfo) -> bool {
    left.pid == right.pid
        && identity_matches(
            &left.name,
            left.exe_path.as_deref(),
            left.start_time,
            &right.name,
            right.exe_path.as_deref(),
            right.start_time,
        )
}

/// Compares process identity for non-destructive snapshot presentation only.
///
/// Windows can withhold both the executable path and creation time for
/// protected processes. Treating every such PID as a new process makes two
/// consecutive snapshots report large, false start/exit counts. This relaxed
/// comparison is deliberately separate from [`same_process`], which remains
/// fail-closed for Close/Kill/Kill Tree validation.
pub fn same_process_for_snapshot(left: &ProcessInfo, right: &ProcessInfo) -> bool {
    if left.pid != right.pid || !left.name.eq_ignore_ascii_case(&right.name) {
        return false;
    }

    match (left.start_time, right.start_time) {
        (Some(left_start), Some(right_start)) => {
            left_start == right_start
                && paths_do_not_conflict(left.exe_path.as_deref(), right.exe_path.as_deref())
        }
        (None, None) => paths_do_not_conflict(left.exe_path.as_deref(), right.exe_path.as_deref()),
        _ => false,
    }
}

pub fn target_list_changed(previous: &[ProcessInfo], current: &[ProcessInfo]) -> bool {
    let mut previous = previous.iter().map(identity_key).collect::<Vec<_>>();
    let mut current = current.iter().map(identity_key).collect::<Vec<_>>();
    previous.sort();
    current.sort();
    previous != current
}

fn identity_matches(
    expected_name: &str,
    expected_exe_path: Option<&str>,
    expected_start_time: Option<SystemTime>,
    current_name: &str,
    current_exe_path: Option<&str>,
    current_start_time: Option<SystemTime>,
) -> bool {
    if !expected_name.eq_ignore_ascii_case(current_name) {
        return false;
    }

    let start_time_matches = match (expected_start_time, current_start_time) {
        (Some(expected), Some(current)) => expected == current,
        _ => false,
    };
    if !start_time_matches {
        return false;
    }

    paths_do_not_conflict(expected_exe_path, current_exe_path)
}

fn paths_do_not_conflict(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.eq_ignore_ascii_case(right),
        _ => true,
    }
}

fn identity_key(process: &ProcessInfo) -> (u32, String, Option<String>, Option<SystemTime>) {
    (
        process.pid,
        process.name.to_lowercase(),
        process.exe_path.as_ref().map(|path| path.to_lowercase()),
        process.start_time,
    )
}

fn pid_reused_error(pid: u32) -> anyhow::Error {
    anyhow!("PID {pid} may now be a different process. Reload and try again.")
}

#[cfg(windows)]
pub(crate) fn process_start_time(pid: u32) -> anyhow::Result<SystemTime> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
            .with_context(|| format!("failed to open PID {pid} for creation-time query"))?;
        let result = process_start_time_from_handle(handle, pid);
        let _ = CloseHandle(handle);
        result
    }
}

#[cfg(windows)]
pub(crate) fn process_start_time_from_handle(
    handle: windows::Win32::Foundation::HANDLE,
    pid: u32,
) -> anyhow::Result<SystemTime> {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::GetProcessTimes;

    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe {
        GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user)
            .with_context(|| format!("failed to read PID {pid} creation time"))?;
    }
    system_time_from_filetime(creation)
}

#[cfg(windows)]
pub(crate) fn system_time_from_filetime(
    creation: windows::Win32::Foundation::FILETIME,
) -> anyhow::Result<SystemTime> {
    const WINDOWS_TO_UNIX_SECONDS: u64 = 11_644_473_600;
    const TICKS_PER_SECOND: u64 = 10_000_000;
    const NANOS_PER_TICK: u64 = 100;

    let ticks = ((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64;
    let unix_ticks = ticks
        .checked_sub(WINDOWS_TO_UNIX_SECONDS * TICKS_PER_SECOND)
        .context("invalid Windows process creation time")?;
    let duration = Duration::from_secs(unix_ticks / TICKS_PER_SECOND)
        + Duration::from_nanos((unix_ticks % TICKS_PER_SECOND) * NANOS_PER_TICK);
    UNIX_EPOCH
        .checked_add(duration)
        .context("Windows process creation time is out of range")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(pid: u32, name: &str, exe_path: Option<&str>, start_time: u64) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: name.to_string(),
            exe_path: exe_path.map(str::to_string),
            start_time: Some(UNIX_EPOCH + Duration::from_secs(start_time)),
            ..Default::default()
        }
    }

    #[test]
    fn detects_same_identity_case_insensitively() {
        let left = process(10, "Python.exe", Some(r"C:\Python\python.exe"), 100);
        let right = process(10, "python.exe", Some(r"c:\python\PYTHON.EXE"), 100);
        assert!(same_process(&left, &right));
    }

    #[test]
    fn detects_reused_pid_when_start_time_differs() {
        let left = process(10, "python.exe", Some(r"C:\Python\python.exe"), 100);
        let right = process(10, "python.exe", Some(r"C:\Python\python.exe"), 101);
        assert!(!same_process(&left, &right));
    }

    #[test]
    fn detects_reused_pid_within_the_same_second() {
        let mut left = process(10, "python.exe", Some(r"C:\Python\python.exe"), 100);
        let mut right = left.clone();
        left.start_time = left.start_time.map(|time| time + Duration::from_nanos(100));
        right.start_time = right
            .start_time
            .map(|time| time + Duration::from_nanos(200));

        assert!(!same_process(&left, &right));
    }

    #[test]
    fn target_list_detects_subsecond_identity_changes() {
        let left = process(10, "python.exe", Some(r"C:\Python\python.exe"), 100);
        let mut right = left.clone();
        right.start_time = right
            .start_time
            .map(|time| time + Duration::from_nanos(100));

        assert!(target_list_changed(&[left], &[right]));
    }

    #[test]
    fn rejects_identity_when_start_time_is_unavailable() {
        let mut left = process(10, "python.exe", Some(r"C:\Python\python.exe"), 100);
        let mut right = left.clone();
        left.start_time = None;
        right.start_time = None;

        assert!(!same_process(&left, &right));
    }

    #[test]
    fn snapshot_identity_keeps_uninspectable_processes_continuous() {
        let mut left = process(10, "System", None, 100);
        let mut right = left.clone();
        left.start_time = None;
        right.start_time = None;

        assert!(same_process_for_snapshot(&left, &right));
        right.name = "Registry".to_string();
        assert!(!same_process_for_snapshot(&left, &right));

        right = left.clone();
        right.start_time = Some(UNIX_EPOCH + Duration::from_secs(100));
        assert!(!same_process_for_snapshot(&left, &right));
    }

    #[test]
    fn snapshot_identity_never_ignores_conflicting_known_metadata() {
        let left = process(10, "python.exe", Some(r"C:\Python\python.exe"), 100);
        let mut right = process(10, "python.exe", Some(r"C:\Other\python.exe"), 100);

        assert!(!same_process_for_snapshot(&left, &right));
        let mut left_without_start = left.clone();
        let mut right_without_start = right.clone();
        left_without_start.start_time = None;
        right_without_start.start_time = None;
        assert!(!same_process_for_snapshot(
            &left_without_start,
            &right_without_start
        ));

        right.exe_path = left.exe_path.clone();
        right.start_time = Some(UNIX_EPOCH + Duration::from_secs(101));
        assert!(!same_process_for_snapshot(&left, &right));
    }

    #[test]
    fn start_time_is_sufficient_when_one_executable_path_is_unavailable() {
        let left = process(10, "python.exe", Some(r"C:\Python\python.exe"), 100);
        let right = process(10, "python.exe", None, 100);

        assert!(same_process(&left, &right));
    }

    #[cfg(windows)]
    #[test]
    fn preserves_windows_creation_time_subsecond_ticks() {
        use windows::Win32::Foundation::FILETIME;

        const WINDOWS_TO_UNIX_SECONDS: u64 = 11_644_473_600;
        const TICKS_PER_SECOND: u64 = 10_000_000;
        let ticks = (WINDOWS_TO_UNIX_SECONDS + 100) * TICKS_PER_SECOND + 123_456;
        let creation = FILETIME {
            dwLowDateTime: ticks as u32,
            dwHighDateTime: (ticks >> 32) as u32,
        };

        let converted = system_time_from_filetime(creation).expect("valid FILETIME");
        assert_eq!(
            converted.duration_since(UNIX_EPOCH).expect("after epoch"),
            Duration::from_secs(100) + Duration::from_nanos(12_345_600)
        );
    }
}
