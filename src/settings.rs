use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    pub refresh_mode: RefreshMode,
    pub auto_refresh_interval_ms: u64,
    pub show_system_processes: bool,
    pub python_only: bool,
    pub gpu_active_only: bool,
    pub codex_related_only: bool,
    pub local_web_only: bool,
    pub heavy_ram_only: bool,
    pub heavy_vram_only: bool,
    pub memory_changed_only: bool,
    pub heavy_ram_threshold_mb: u64,
    pub heavy_vram_threshold_mb: u64,
    pub table_view: TableView,
    pub default_sort: String,
    pub protected_process_names: Vec<String>,
    pub python_keywords: Vec<String>,
    pub codex_root_keywords: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RefreshMode {
    Manual,
    Auto,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum TableView {
    #[default]
    Compact,
    Advanced,
}

impl std::fmt::Display for TableView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::Compact => "Compact",
            Self::Advanced => "Advanced",
        };
        f.write_str(text)
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            refresh_mode: RefreshMode::Manual,
            auto_refresh_interval_ms: 5000,
            show_system_processes: false,
            python_only: false,
            gpu_active_only: false,
            codex_related_only: false,
            local_web_only: false,
            heavy_ram_only: false,
            heavy_vram_only: false,
            memory_changed_only: false,
            heavy_ram_threshold_mb: 1024,
            heavy_vram_threshold_mb: 1024,
            table_view: TableView::Compact,
            default_sort: "VramDesc".to_string(),
            protected_process_names: vec!["dwm.exe", "explorer.exe"]
                .into_iter()
                .map(String::from)
                .collect(),
            python_keywords: vec![
                "python.exe",
                "pythonw.exe",
                "py.exe",
                ".py",
                "uvicorn",
                "streamlit",
                "gradio",
                "jupyter",
                "ipython",
                "conda",
                "comfyui",
                "forge",
                "stable-diffusion-webui",
                "launch.py",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            codex_root_keywords: vec![
                "codex",
                "openai",
                "wt.exe",
                "windowsterminal.exe",
                "cmd.exe",
                "powershell.exe",
                "pwsh.exe",
                "code.exe",
                "wsl.exe",
                "claude",
                "claude.exe",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
        }
    }
}

impl Settings {
    pub fn default_path() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|parent| parent.join("settings.json")))
            .unwrap_or_else(|| PathBuf::from("settings.json"))
    }

    pub fn load_or_default(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)?;
        let settings = serde_json::from_str(&text)?;
        Ok(settings)
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec_pretty(self)?;
        write_atomic(path, &bytes)
    }

    pub fn auto_refresh_enabled(&self) -> bool {
        self.refresh_mode == RefreshMode::Auto
    }

    pub fn set_auto_refresh_enabled(&mut self, enabled: bool) {
        self.refresh_mode = if enabled {
            RefreshMode::Auto
        } else {
            RefreshMode::Manual
        };
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let (temporary_path, mut temporary_file) = create_temporary_file(path)?;
    let write_result = (|| {
        temporary_file
            .write_all(bytes)
            .with_context(|| format!("failed to write {}", temporary_path.to_string_lossy()))?;
        temporary_file
            .sync_all()
            .with_context(|| format!("failed to flush {}", temporary_path.to_string_lossy()))?;
        Ok::<_, anyhow::Error>(())
    })();
    drop(temporary_file);
    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(error);
    }

    if let Err(error) = replace_file(&temporary_path, path) {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(error);
    }
    Ok(())
}

fn create_temporary_file(path: &Path) -> anyhow::Result<(PathBuf, std::fs::File)> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    for attempt in 0..100_u32 {
        let temporary_path = parent.join(format!(
            ".runscope-settings-{}-{attempt}.tmp",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
        {
            Ok(file) => return Ok((temporary_path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to create {}", temporary_path.to_string_lossy())
                });
            }
        }
    }
    bail!(
        "failed to create a unique temporary settings file next to {}",
        path.to_string_lossy()
    )
}

#[cfg(windows)]
fn replace_file(temporary_path: &Path, path: &Path) -> anyhow::Result<()> {
    use std::iter::once;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let temporary_wide = temporary_path
        .as_os_str()
        .encode_wide()
        .chain(once(0))
        .collect::<Vec<_>>();
    let target_wide = path
        .as_os_str()
        .encode_wide()
        .chain(once(0))
        .collect::<Vec<_>>();
    unsafe {
        MoveFileExW(
            PCWSTR(temporary_wide.as_ptr()),
            PCWSTR(target_wide.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .with_context(|| format!("failed to atomically replace {}", path.to_string_lossy()))
}

#[cfg(not(windows))]
fn replace_file(temporary_path: &Path, path: &Path) -> anyhow::Result<()> {
    std::fs::rename(temporary_path, path)
        .with_context(|| format!("failed to atomically replace {}", path.to_string_lossy()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_old_settings_without_new_ui_fields() {
        let json = r#"{
          "refresh_mode": "manual",
          "auto_refresh_interval_ms": 5000,
          "show_system_processes": false,
          "python_only": true,
          "gpu_active_only": false,
          "codex_related_only": false,
          "default_sort": "VramDesc",
          "protected_process_names": ["System"],
          "python_keywords": ["python.exe"],
          "codex_root_keywords": ["codex"]
        }"#;

        let settings: Settings = serde_json::from_str(json).expect("old settings should parse");

        assert!(settings.python_only);
        assert_eq!(settings.table_view, TableView::Compact);
        assert!(!settings.local_web_only);
        assert!(!settings.heavy_ram_only);
        assert!(!settings.heavy_vram_only);
        assert!(!settings.memory_changed_only);
        assert_eq!(settings.heavy_ram_threshold_mb, 1024);
        assert_eq!(settings.heavy_vram_threshold_mb, 1024);
    }

    #[test]
    fn atomically_replaces_existing_settings() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("current time")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "runscope-settings-test-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).expect("create test directory");
        let path = directory.join("settings.json");

        let original = Settings::default();
        original.save(&path).expect("save original settings");
        let updated = Settings {
            python_only: true,
            protected_process_names: vec!["important.exe".to_string()],
            ..original
        };
        updated.save(&path).expect("replace settings");

        assert_eq!(
            Settings::load_or_default(&path).expect("reload settings"),
            updated
        );
        assert_eq!(
            std::fs::read_dir(&directory)
                .expect("read test directory")
                .count(),
            1,
            "temporary file should be removed after replacement"
        );

        std::fs::remove_file(&path).expect("remove test settings");
        std::fs::remove_dir(&directory).expect("remove test directory");
    }
}
