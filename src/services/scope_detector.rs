use crate::model::{ProcessInfo, ProcessScope};
use crate::services::protection;
use crate::settings::Settings;
use std::collections::HashMap;

pub fn assign_scopes(processes: &mut [ProcessInfo], settings: &Settings) {
    let by_pid: HashMap<u32, ProcessInfo> = processes.iter().map(|p| (p.pid, p.clone())).collect();
    let current_pid = std::process::id();
    for p in processes {
        p.protection_reason = None;
        if p.pid == current_pid {
            p.protected = true;
            p.protection_reason = Some("Current RunScope process".to_string());
        } else if protection::is_protected(&p.name, settings) {
            p.protected = true;
            p.protection_reason = Some("Protected process name".to_string());
        } else {
            p.protected = false;
        }
        p.python_related = is_python_related(p, settings);
        p.codex_related = is_codex_related(p, &by_pid, settings);
        let gpu_active = p.is_gpu_active();
        p.scope = if p.protected {
            ProcessScope::Protected
        } else if p.codex_related && gpu_active {
            ProcessScope::CodexGpu
        } else if p.codex_related {
            ProcessScope::CodexTerminal
        } else if gpu_active {
            ProcessScope::GpuActive
        } else if p.python_related {
            ProcessScope::Python
        } else {
            ProcessScope::Normal
        };
    }
}

fn text_of(p: &ProcessInfo) -> String {
    format!(
        "{} {} {}",
        p.name,
        p.exe_path.clone().unwrap_or_default(),
        p.command_line.clone().unwrap_or_default()
    )
    .to_lowercase()
}

fn is_python_related(p: &ProcessInfo, settings: &Settings) -> bool {
    let text = text_of(p);
    let process_name = p.name.to_lowercase();
    let exe_path = p.exe_path.as_deref().unwrap_or_default().to_lowercase();
    let python_executable = matches!(
        process_name.as_str(),
        "python.exe" | "pythonw.exe" | "py.exe" | "python" | "pythonw" | "py"
    ) || exe_path.ends_with("\\python.exe")
        || exe_path.ends_with("\\pythonw.exe")
        || exe_path.ends_with("\\py.exe");

    for keyword in &settings.python_keywords {
        let keyword = keyword.to_lowercase();
        if keyword == ".py" {
            if python_executable && text.contains(".py") {
                return true;
            }
            continue;
        }
        if text.contains(&keyword) {
            return true;
        }
    }

    python_executable
}

fn is_root_candidate(p: &ProcessInfo, settings: &Settings) -> bool {
    let text = text_of(p);
    settings
        .codex_root_keywords
        .iter()
        .any(|kw| text.contains(&kw.to_lowercase()))
}

fn is_codex_related(
    p: &ProcessInfo,
    by_pid: &HashMap<u32, ProcessInfo>,
    settings: &Settings,
) -> bool {
    let mut current = Some(p.clone());
    for _ in 0..40 {
        let Some(cur) = current else {
            return false;
        };
        if is_root_candidate(&cur, settings) {
            return true;
        }
        current = cur.parent_pid.and_then(|pid| by_pid.get(&pid).cloned());
    }
    false
}
