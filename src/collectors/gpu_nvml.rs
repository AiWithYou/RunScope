use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr};
use std::ptr::null_mut;

use anyhow::{bail, Context};
use windows::core::{w, PCSTR};
use windows::Win32::Foundation::{FreeLibrary, HANDLE, HMODULE};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
};

use crate::model::GpuProcessType;

#[derive(Debug, Clone)]
pub struct VramUsage {
    pub bytes: u64,
    pub device_indices: Vec<u32>,
    pub device_names: Vec<String>,
    pub process_type: GpuProcessType,
}

#[derive(Debug, Clone)]
struct DeviceVramUsage {
    bytes: u64,
    device_name: String,
    process_type: GpuProcessType,
}

type NvmlReturn = i32;
type NvmlDevice = *mut c_void;

const NVML_SUCCESS: NvmlReturn = 0;
const NVML_ERROR_NOT_SUPPORTED: NvmlReturn = 3;
const NVML_ERROR_INSUFFICIENT_SIZE: NvmlReturn = 7;
const NVML_VALUE_NOT_AVAILABLE: u64 = u64::MAX;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct NvmlProcessInfo {
    pid: u32,
    used_gpu_memory: u64,
    gpu_instance_id: u32,
    compute_instance_id: u32,
}

type NvmlInitV2 = unsafe extern "C" fn() -> NvmlReturn;
type NvmlShutdown = unsafe extern "C" fn() -> NvmlReturn;
type NvmlDeviceGetCountV2 = unsafe extern "C" fn(*mut u32) -> NvmlReturn;
type NvmlDeviceGetHandleByIndexV2 = unsafe extern "C" fn(u32, *mut NvmlDevice) -> NvmlReturn;
type NvmlDeviceGetName = unsafe extern "C" fn(NvmlDevice, *mut c_char, u32) -> NvmlReturn;
type NvmlDeviceGetRunningProcesses =
    unsafe extern "C" fn(NvmlDevice, *mut u32, *mut NvmlProcessInfo) -> NvmlReturn;

struct NvmlLibrary {
    module: HMODULE,
}

impl NvmlLibrary {
    fn load() -> anyhow::Result<Self> {
        let module = unsafe {
            LoadLibraryExW(
                w!("nvml.dll"),
                HANDLE::default(),
                LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        }
        .context("nvml.dll not found in Windows System32")?;
        Ok(Self { module })
    }

    unsafe fn symbol<T: Copy>(&self, name: &'static [u8]) -> anyhow::Result<T> {
        let symbol = GetProcAddress(self.module, PCSTR(name.as_ptr()));
        let Some(symbol) = symbol else {
            let clean_name = String::from_utf8_lossy(&name[..name.len().saturating_sub(1)]);
            bail!("NVML symbol not found: {clean_name}");
        };
        Ok(std::mem::transmute_copy(&symbol))
    }
}

impl Drop for NvmlLibrary {
    fn drop(&mut self) {
        unsafe {
            let _ = FreeLibrary(self.module);
        }
    }
}

pub fn collect_vram_by_pid_nvml() -> anyhow::Result<HashMap<u32, VramUsage>> {
    let nvml = NvmlLibrary::load()?;

    unsafe {
        let init: NvmlInitV2 = nvml.symbol(b"nvmlInit_v2\0")?;
        let shutdown: NvmlShutdown = nvml.symbol(b"nvmlShutdown\0")?;
        let device_count: NvmlDeviceGetCountV2 = nvml.symbol(b"nvmlDeviceGetCount_v2\0")?;
        let handle_by_index: NvmlDeviceGetHandleByIndexV2 =
            nvml.symbol(b"nvmlDeviceGetHandleByIndex_v2\0")?;
        let device_name: NvmlDeviceGetName = nvml.symbol(b"nvmlDeviceGetName\0")?;
        let compute_processes: NvmlDeviceGetRunningProcesses =
            nvml.symbol(b"nvmlDeviceGetComputeRunningProcesses_v3\0")?;
        let graphics_processes: NvmlDeviceGetRunningProcesses =
            nvml.symbol(b"nvmlDeviceGetGraphicsRunningProcesses_v3\0")?;

        check(init(), "nvmlInit_v2")?;

        let collect_result = collect_devices(
            device_count,
            handle_by_index,
            device_name,
            compute_processes,
            graphics_processes,
        );
        let _ = shutdown();
        collect_result
    }
}

unsafe fn collect_devices(
    device_count: NvmlDeviceGetCountV2,
    handle_by_index: NvmlDeviceGetHandleByIndexV2,
    device_name: NvmlDeviceGetName,
    compute_processes: NvmlDeviceGetRunningProcesses,
    graphics_processes: NvmlDeviceGetRunningProcesses,
) -> anyhow::Result<HashMap<u32, VramUsage>> {
    let mut count = 0;
    check(device_count(&mut count), "nvmlDeviceGetCount_v2")?;
    let mut per_device = HashMap::new();

    for index in 0..count {
        let mut device: NvmlDevice = null_mut();
        if handle_by_index(index, &mut device) != NVML_SUCCESS {
            continue;
        }
        let name = read_device_name(device_name, device).unwrap_or_else(|| "NVIDIA".to_string());

        for info in read_processes(device, compute_processes, "compute")? {
            add_device_usage(&mut per_device, index, info, &name, GpuProcessType::Compute);
        }
        for info in read_processes(device, graphics_processes, "graphics")? {
            add_device_usage(
                &mut per_device,
                index,
                info,
                &name,
                GpuProcessType::Graphics,
            );
        }
    }

    Ok(collapse_device_usage(per_device))
}

unsafe fn read_device_name(device_name: NvmlDeviceGetName, device: NvmlDevice) -> Option<String> {
    let mut buffer = [0 as c_char; 96];
    if device_name(device, buffer.as_mut_ptr(), buffer.len() as u32) != NVML_SUCCESS {
        return None;
    }
    Some(
        CStr::from_ptr(buffer.as_ptr())
            .to_string_lossy()
            .to_string(),
    )
}

unsafe fn read_processes(
    device: NvmlDevice,
    reader: NvmlDeviceGetRunningProcesses,
    label: &str,
) -> anyhow::Result<Vec<NvmlProcessInfo>> {
    let mut count = 0;
    let first_result = reader(device, &mut count, null_mut());
    if first_result == NVML_ERROR_NOT_SUPPORTED {
        return Ok(Vec::new());
    }
    if first_result != NVML_SUCCESS && first_result != NVML_ERROR_INSUFFICIENT_SIZE {
        bail!("NVML {label} process size query failed with code {first_result}");
    }
    if count == 0 {
        return Ok(Vec::new());
    }

    for _ in 0..3 {
        let mut processes = vec![NvmlProcessInfo::default(); count as usize];
        let result = reader(device, &mut count, processes.as_mut_ptr());
        if result == NVML_SUCCESS {
            processes.truncate(count as usize);
            return Ok(processes);
        }
        if result == NVML_ERROR_NOT_SUPPORTED {
            return Ok(Vec::new());
        }
        if result != NVML_ERROR_INSUFFICIENT_SIZE {
            bail!("NVML {label} process query failed with code {result}");
        }
    }
    bail!("NVML {label} process list kept resizing during collection")
}

fn add_device_usage(
    result: &mut HashMap<(u32, u32), DeviceVramUsage>,
    device_index: u32,
    info: NvmlProcessInfo,
    device_name: &str,
    process_type: GpuProcessType,
) {
    if info.pid == 0
        || info.used_gpu_memory == 0
        || info.used_gpu_memory == NVML_VALUE_NOT_AVAILABLE
    {
        return;
    }

    result
        .entry((info.pid, device_index))
        .and_modify(|current| {
            current.bytes = current.bytes.max(info.used_gpu_memory);
            current.process_type = merge_process_type(current.process_type, process_type);
        })
        .or_insert_with(|| DeviceVramUsage {
            bytes: info.used_gpu_memory,
            device_name: device_name.to_string(),
            process_type,
        });
}

fn collapse_device_usage(
    per_device: HashMap<(u32, u32), DeviceVramUsage>,
) -> HashMap<u32, VramUsage> {
    let mut result = HashMap::new();
    for ((pid, device_index), usage) in per_device {
        result
            .entry(pid)
            .and_modify(|current: &mut VramUsage| {
                current.bytes = current.bytes.saturating_add(usage.bytes);
                current.device_indices.push(device_index);
                current.device_names.push(usage.device_name.clone());
                current.process_type = merge_process_type(current.process_type, usage.process_type);
            })
            .or_insert_with(|| VramUsage {
                bytes: usage.bytes,
                device_indices: vec![device_index],
                device_names: vec![usage.device_name],
                process_type: usage.process_type,
            });
    }
    for usage in result.values_mut() {
        usage.device_indices.sort_unstable();
        usage.device_indices.dedup();
        usage.device_names.sort();
        usage.device_names.dedup();
    }
    result
}

fn merge_process_type(current: GpuProcessType, next: GpuProcessType) -> GpuProcessType {
    match (current, next) {
        (GpuProcessType::Compute, GpuProcessType::Graphics)
        | (GpuProcessType::Graphics, GpuProcessType::Compute)
        | (GpuProcessType::Both, _)
        | (_, GpuProcessType::Both) => GpuProcessType::Both,
        (GpuProcessType::Unknown, value) | (value, GpuProcessType::Unknown) => value,
        (value, _) => value,
    }
}

fn check(code: NvmlReturn, operation: &str) -> anyhow::Result<()> {
    if code == NVML_SUCCESS {
        Ok(())
    } else {
        bail!("{operation} failed with NVML code {code}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_and_graphics_rows_on_same_device_are_not_double_counted() {
        let mut per_device = HashMap::new();
        let info = NvmlProcessInfo {
            pid: 42,
            used_gpu_memory: 100,
            ..Default::default()
        };
        add_device_usage(&mut per_device, 0, info, "GPU A", GpuProcessType::Compute);
        add_device_usage(&mut per_device, 0, info, "GPU A", GpuProcessType::Graphics);
        add_device_usage(
            &mut per_device,
            1,
            NvmlProcessInfo {
                used_gpu_memory: 50,
                ..info
            },
            "GPU B",
            GpuProcessType::Compute,
        );

        let usage = collapse_device_usage(per_device).remove(&42).unwrap();
        assert_eq!(usage.bytes, 150);
        assert_eq!(usage.device_indices, vec![0, 1]);
        assert_eq!(usage.device_names, vec!["GPU A", "GPU B"]);
        assert_eq!(usage.process_type, GpuProcessType::Both);
    }
}
