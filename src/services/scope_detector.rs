use crate::model::{ProcessInfo, ProcessScope};
use crate::services::protection;
use crate::settings::Settings;
use std::collections::HashMap;

pub fn assign_scopes(processes: &mut [ProcessInfo], settings: &Settings) {
    let ancestry = processes
        .iter()
        .map(|process| {
            (
                process.pid,
                AncestryNode {
                    parent_pid: process.parent_pid,
                    root_candidate: is_root_candidate(process, settings),
                },
            )
        })
        .collect::<HashMap<_, _>>();
    let current_pid = std::process::id();
    for p in processes {
        p.protection_reason = None;
        if p.pid == current_pid {
            p.protected = true;
            p.protection_reason = Some("Current RunScope process".to_string());
        } else if protection::is_critical(&p.name) {
            p.protected = true;
            p.protection_reason = Some("Built-in critical process".to_string());
        } else if protection::is_configured(&p.name, settings) {
            p.protected = true;
            p.protection_reason = Some("Configured protected process name".to_string());
        } else {
            p.protected = false;
        }
        p.python_related = is_python_related(p, settings);
        p.codex_related = is_codex_related(p.pid, &ancestry);
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

#[derive(Debug, Clone, Copy)]
struct AncestryNode {
    parent_pid: Option<u32>,
    root_candidate: bool,
}

fn text_of(p: &ProcessInfo) -> String {
    format!(
        "{} {} {}",
        p.name,
        p.exe_path.as_deref().unwrap_or_default(),
        p.command_line.as_deref().unwrap_or_default()
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

fn is_codex_related(pid: u32, ancestry: &HashMap<u32, AncestryNode>) -> bool {
    let mut current_pid = Some(pid);
    for _ in 0..40 {
        let Some(pid) = current_pid else {
            return false;
        };
        let Some(current) = ancestry.get(&pid) else {
            return false;
        };
        if current.root_candidate {
            return true;
        }
        current_pid = current.parent_pid;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn propagates_root_classification_to_descendants_without_process_clones() {
        let settings = Settings {
            codex_root_keywords: vec!["terminal.exe".to_string()],
            ..Default::default()
        };
        let mut processes = vec![
            ProcessInfo {
                pid: 10,
                name: "terminal.exe".to_string(),
                ..Default::default()
            },
            ProcessInfo {
                pid: 20,
                parent_pid: Some(10),
                name: "worker.exe".to_string(),
                ..Default::default()
            },
        ];

        assign_scopes(&mut processes, &settings);

        assert!(processes[0].codex_related);
        assert!(processes[1].codex_related);
        assert_eq!(processes[1].scope, ProcessScope::CodexTerminal);
    }

    #[test]
    fn built_in_critical_process_is_protected_with_empty_user_list() {
        let mut settings = Settings::default();
        settings.protected_process_names.clear();
        let mut processes = vec![ProcessInfo {
            pid: 999_999,
            name: "lsass.exe".to_string(),
            ..Default::default()
        }];

        assign_scopes(&mut processes, &settings);

        assert!(processes[0].protected);
        assert_eq!(
            processes[0].protection_reason.as_deref(),
            Some("Built-in critical process")
        );
    }
}
