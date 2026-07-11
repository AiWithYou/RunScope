use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(path, text)?;
        Ok(())
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
}
