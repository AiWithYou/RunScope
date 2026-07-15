use crate::model::{ProcessInfo, ProcessScope};
use crate::services::protection;
use crate::settings::Settings;
use std::collections::HashMap;

pub fn assign_scopes(processes: &mut [ProcessInfo], settings: &Settings) {
    let python_keywords = lowercase_keywords(&settings.python_keywords);
    let codex_root_keywords = lowercase_keywords(&settings.codex_root_keywords);
    let mut python_related = Vec::with_capacity(processes.len());
    let ancestry = processes
        .iter()
        .map(|process| {
            let text = text_of(process);
            python_related.push(is_python_related(process, &text, &python_keywords));
            (
                process.pid,
                AncestryNode {
                    parent_pid: process.parent_pid,
                    root_candidate: is_root_candidate(&text, &codex_root_keywords),
                },
            )
        })
        .collect::<HashMap<_, _>>();
    let current_pid = std::process::id();
    for (p, python_related) in processes.iter_mut().zip(python_related) {
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
        p.python_related = python_related;
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
    let exe_path = p.exe_path.as_deref().unwrap_or_default();
    let command_line = p.command_line.as_deref().unwrap_or_default();
    let mut text = String::with_capacity(p.name.len() + exe_path.len() + command_line.len() + 2);
    text.push_str(&p.name);
    text.push(' ');
    text.push_str(exe_path);
    text.push(' ');
    text.push_str(command_line);
    if text.is_ascii() {
        text.make_ascii_lowercase();
        text
    } else {
        text.to_lowercase()
    }
}

fn is_python_related(p: &ProcessInfo, text: &str, python_keywords: &[String]) -> bool {
    let python_executable = [
        "python.exe",
        "pythonw.exe",
        "py.exe",
        "python",
        "pythonw",
        "py",
    ]
    .iter()
    .any(|name| p.name.eq_ignore_ascii_case(name))
        || [r"\python.exe", r"\pythonw.exe", r"\py.exe"]
            .iter()
            .any(|suffix| {
                ends_with_ignore_ascii_case(p.exe_path.as_deref().unwrap_or_default(), suffix)
            });

    for keyword in python_keywords {
        if keyword == ".py" {
            if python_executable && text.contains(".py") {
                return true;
            }
            continue;
        }
        if text.contains(keyword) {
            return true;
        }
    }

    python_executable
}

fn is_root_candidate(text: &str, codex_root_keywords: &[String]) -> bool {
    codex_root_keywords
        .iter()
        .any(|keyword| text.contains(keyword))
}

fn lowercase_keywords(keywords: &[String]) -> Vec<String> {
    keywords
        .iter()
        .map(|keyword| keyword.to_lowercase())
        .collect()
}

fn ends_with_ignore_ascii_case(value: &str, suffix: &str) -> bool {
    value
        .get(value.len().saturating_sub(suffix.len())..)
        .is_some_and(|ending| ending.eq_ignore_ascii_case(suffix))
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

    #[test]
    fn keyword_detection_remains_case_insensitive() {
        let settings = Settings {
            python_keywords: vec!["UVICORN".to_string()],
            codex_root_keywords: vec!["TERMINAL.EXE".to_string()],
            ..Default::default()
        };
        let mut processes = vec![
            ProcessInfo {
                pid: 10,
                name: "Terminal.exe".to_string(),
                ..Default::default()
            },
            ProcessInfo {
                pid: 20,
                parent_pid: Some(10),
                name: "worker.exe".to_string(),
                command_line: Some("uvicorn app:api".to_string()),
                ..Default::default()
            },
        ];

        assign_scopes(&mut processes, &settings);

        assert!(processes[1].python_related);
        assert!(processes[1].codex_related);
    }
}
