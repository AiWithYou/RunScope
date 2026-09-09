use crate::collectors::gpu_nvml::VramUsage;
use crate::model::GpuProcessType;
use anyhow::{bail, Context};
use std::collections::HashMap;
use std::sync::{mpsc, OnceLock};
use std::time::Duration;

/// One bounded worker owns the native PDH query. A stalled driver cannot create
/// an unbounded succession of threads or freeze the UI/recorder indefinitely.
pub fn collect_vram_by_pid_windows_perf() -> anyhow::Result<HashMap<u32, VramUsage>> {
    type Reply = mpsc::Sender<anyhow::Result<HashMap<u32, VramUsage>>>;
    static WORKER: OnceLock<Result<mpsc::SyncSender<Reply>, String>> = OnceLock::new();
    let worker = WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel::<Reply>(1);
        std::thread::Builder::new()
            .name("runscope-pdh".into())
            .spawn(move || {
                let mut query = None;
                while let Ok(reply) = rx.recv() {
                    let result = (|| {
                        if query.is_none() {
                            query = Some(Query::new()?);
                        }
                        query.as_mut().expect("query initialized").collect()
                    })();
                    if result.is_err() {
                        query = None;
                    }
                    let _ = reply.send(result);
                }
            })
            .map(|_| tx)
            .map_err(|error| error.to_string())
    });
    let worker = worker
        .as_ref()
        .map_err(|error| anyhow::anyhow!("PDH worker: {error}"))?;
    let (tx, rx) = mpsc::channel();
    worker
        .try_send(tx)
        .context("PDH worker busy or disconnected")?;
    rx.recv_timeout(Duration::from_millis(1000))
        .context("PDH query timed out or disconnected")?
}

struct Query {
    handle: isize,
    counter: isize,
}

impl Query {
    fn new() -> anyhow::Result<Self> {
        use windows::core::{w, PCWSTR};
        use windows::Win32::System::Performance::{PdhAddEnglishCounterW, PdhOpenQueryW};
        let mut handle = 0;
        check(
            unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut handle) },
            "PdhOpenQueryW",
        )?;
        let mut query = Self { handle, counter: 0 };
        // Language-neutral counter names: unlike typeperf, works on non-English Windows.
        check(
            unsafe {
                PdhAddEnglishCounterW(
                    handle,
                    w!(r"\GPU Process Memory(*)\Dedicated Usage"),
                    0,
                    &mut query.counter,
                )
            },
            "PdhAddEnglishCounterW",
        )?;
        Ok(query)
    }

    fn collect(&mut self) -> anyhow::Result<HashMap<u32, VramUsage>> {
        use windows::Win32::System::Performance::{
            PdhCollectQueryData, PdhGetRawCounterArrayW, PDH_RAW_COUNTER_ITEM_W,
        };
        const MORE_DATA: u32 = 0x800007D2; // PDH_MORE_DATA, pdhmsg.h
        check(
            unsafe { PdhCollectQueryData(self.handle) },
            "PdhCollectQueryData",
        )?;
        let mut bytes = 0;
        let mut count = 0;
        let status = unsafe { PdhGetRawCounterArrayW(self.counter, &mut bytes, &mut count, None) };
        if status != MORE_DATA {
            check(status, "PdhGetRawCounterArrayW(size)")?;
        }
        if bytes == 0 {
            return Ok(HashMap::new());
        }
        for _ in 0..3 {
            if bytes > 16 * 1024 * 1024 {
                bail!("PDH array exceeds 16 MiB");
            }
            // Typed allocation guarantees alignment, including the trailing UTF-16 names.
            let item_size = std::mem::size_of::<PDH_RAW_COUNTER_ITEM_W>();
            let slots = (bytes as usize).div_ceil(item_size);
            let mut buffer = vec![PDH_RAW_COUNTER_ITEM_W::default(); slots];
            let capacity_bytes = slots * item_size;
            bytes = capacity_bytes as u32;
            let status = unsafe {
                PdhGetRawCounterArrayW(
                    self.counter,
                    &mut bytes,
                    &mut count,
                    Some(buffer.as_mut_ptr()),
                )
            };
            if status == MORE_DATA {
                continue;
            }
            check(status, "PdhGetRawCounterArrayW")?;
            if count as usize > slots {
                bail!("PDH returned an invalid item count");
            }
            let begin = buffer.as_ptr() as usize;
            let end = begin + capacity_bytes;
            let mut map = HashMap::new();
            for item in buffer.iter().take(count as usize) {
                if item.RawValue.CStatus > 1 || item.RawValue.FirstValue <= 0 {
                    continue;
                }
                let address = item.szName.0 as usize;
                if address < begin || address >= end || address % 2 != 0 {
                    continue;
                }
                let name_units =
                    unsafe { std::slice::from_raw_parts(item.szName.0, (end - address) / 2) };
                let Some(length) = name_units.iter().position(|unit| *unit == 0) else {
                    continue;
                };
                let name = String::from_utf16_lossy(&name_units[..length]);
                let Some(pid) = pid_from_instance(&name) else {
                    continue;
                };
                let bytes = item.RawValue.FirstValue as u64;
                map.entry(pid)
                    .and_modify(|usage: &mut VramUsage| {
                        usage.bytes = usage.bytes.saturating_add(bytes)
                    })
                    .or_insert_with(|| VramUsage {
                        bytes,
                        device_indices: Vec::new(),
                        device_names: vec!["Windows GPU Process Memory (PDH)".into()],
                        process_type: GpuProcessType::Unknown,
                    });
            }
            return Ok(map);
        }
        bail!("PDH array kept resizing")
    }
}

impl Drop for Query {
    fn drop(&mut self) {
        unsafe {
            windows::Win32::System::Performance::PdhCloseQuery(self.handle);
        }
    }
}

fn check(status: u32, operation: &str) -> anyhow::Result<()> {
    if status != 0 {
        bail!("{operation}: PDH status 0x{status:08X}");
    }
    Ok(())
}

fn pid_from_instance(name: &str) -> Option<u32> {
    let rest = name.strip_prefix("pid_")?;
    let (pid, _) = rest.split_once('_')?;
    pid.parse().ok().filter(|pid| *pid != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_pid_independent_of_localized_object_name() {
        assert_eq!(
            pid_from_instance("pid_34728_luid_0x00000000_0x0001179E_phys_0"),
            Some(34728)
        );
        assert_eq!(pid_from_instance("pid_0_luid_0"), None);
        assert_eq!(pid_from_instance("pid_bad_luid_0"), None);
        assert_eq!(pid_from_instance("unrelated_34728"), None);
    }
}
