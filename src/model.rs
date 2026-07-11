use std::time::SystemTime;

#[derive(Debug, Clone, Default)]
pub struct ProcessInfo {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub parent_name: Option<String>,
    pub name: String,
    pub exe_path: Option<String>,
    pub command_line: Option<String>,
    pub cwd: Option<String>,
    pub start_time: Option<SystemTime>,
    pub ram_bytes: u64,
    pub virtual_memory_bytes: u64,
    pub ram_delta_bytes: Option<i64>,
    pub vram_delta_bytes: Option<i64>,
    pub snapshot_state: SnapshotState,
    pub gpu: Option<GpuProcessInfo>,
    pub local_endpoints: Vec<ListeningEndpoint>,
    pub children: Vec<u32>,
    pub scope: ProcessScope,
    pub protected: bool,
    pub protection_reason: Option<String>,
    pub python_related: bool,
    pub codex_related: bool,
    pub searchable_text_lower: String,
}

impl ProcessInfo {
    pub fn vram_bytes(&self) -> Option<u64> {
        self.gpu.as_ref().and_then(|gpu| gpu.vram_bytes)
    }

    pub fn is_gpu_active(&self) -> bool {
        self.gpu.is_some()
    }

    pub fn refresh_searchable_text(&mut self) {
        self.searchable_text_lower = self.build_searchable_text_lower();
    }

    fn build_searchable_text_lower(&self) -> String {
        format!(
            "{} {} {} {} {} {} {} {} {}",
            self.scope,
            self.pid,
            self.name,
            self.parent_name.as_deref().unwrap_or_default(),
            self.exe_path.as_deref().unwrap_or_default(),
            self.command_line.as_deref().unwrap_or_default(),
            self.protection_reason.as_deref().unwrap_or_default(),
            self.local_web_summary(),
            self.snapshot_state
        )
        .to_lowercase()
    }

    pub fn local_web_summary(&self) -> String {
        let mut urls = self
            .local_endpoints
            .iter()
            .map(|endpoint| endpoint.url.clone())
            .collect::<Vec<_>>();
        urls.sort();
        urls.dedup();
        urls.join(", ")
    }

    pub fn local_web_port_count(&self) -> usize {
        let mut ports = self
            .local_endpoints
            .iter()
            .map(|endpoint| endpoint.port)
            .collect::<Vec<_>>();
        ports.sort_unstable();
        ports.dedup();
        ports.len()
    }

    pub fn primary_local_web_url(&self) -> Option<&str> {
        let priority_ports = [7860, 8188, 3000, 5000, 8000, 8080, 5173, 11434];
        for port in priority_ports {
            if let Some(endpoint) = self
                .local_endpoints
                .iter()
                .find(|endpoint| endpoint.port == port)
            {
                return Some(endpoint.url.as_str());
            }
        }
        self.local_endpoints
            .iter()
            .min_by_key(|endpoint| endpoint.port)
            .map(|endpoint| endpoint.url.as_str())
    }

    pub fn local_web_table_text(&self) -> String {
        let Some(primary_url) = self.primary_local_web_url() else {
            return "-".to_string();
        };
        match self.local_web_port_count() {
            0 | 1 => primary_url.to_string(),
            count => format!("{primary_url} (+{})", count - 1),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SnapshotState {
    #[default]
    Unavailable,
    New,
    Changed,
    Unchanged,
}

impl std::fmt::Display for SnapshotState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::Unavailable => "No baseline",
            Self::New => "New",
            Self::Changed => "Changed",
            Self::Unchanged => "Unchanged",
        };
        f.write_str(text)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ListeningEndpoint {
    pub bind_address: String,
    pub port: u16,
    pub url: String,
}

impl ListeningEndpoint {
    pub fn new(bind_address: String, port: u16, ipv6: bool) -> Self {
        let scheme = if matches!(port, 443 | 8443) {
            "https"
        } else {
            "http"
        };
        let url_host = if ipv6 {
            if matches!(
                bind_address.as_str(),
                "::" | "::1" | "0:0:0:0:0:0:0:0" | "0:0:0:0:0:0:0:1"
            ) {
                "[::1]".to_string()
            } else {
                format!("[{bind_address}]")
            }
        } else if bind_address == "0.0.0.0" {
            "127.0.0.1".to_string()
        } else {
            bind_address.clone()
        };
        let url = format!("{scheme}://{url_host}:{port}");

        Self {
            bind_address,
            port,
            url,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct GpuProcessInfo {
    pub device_indices: Vec<u32>,
    pub device_names: Vec<String>,
    pub vram_bytes: Option<u64>,
    pub process_type: GpuProcessType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProcessScope {
    CodexTerminal,
    CodexGpu,
    Python,
    GpuActive,
    Protected,
    #[default]
    Normal,
}

impl std::fmt::Display for ProcessScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::CodexTerminal => "Codex/Terminal",
            Self::CodexGpu => "Codex+GPU",
            Self::Python => "Python",
            Self::GpuActive => "GPU",
            Self::Protected => "Protected",
            Self::Normal => "Normal",
        };
        f.write_str(text)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GpuProcessType {
    Compute,
    Graphics,
    Both,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortPreset {
    RamAsc,
    RamDesc,
    VramAsc,
    #[default]
    VramDesc,
    NameAsc,
    NameDesc,
    PidAsc,
    PidDesc,
    AgeNewest,
    AgeOldest,
    RamGrowth,
    VramGrowth,
}

impl SortPreset {
    pub const ALL: [Self; 12] = [
        Self::VramDesc,
        Self::RamDesc,
        Self::VramGrowth,
        Self::RamGrowth,
        Self::NameAsc,
        Self::NameDesc,
        Self::PidAsc,
        Self::PidDesc,
        Self::AgeNewest,
        Self::AgeOldest,
        Self::VramAsc,
        Self::RamAsc,
    ];

    pub fn from_settings(value: &str) -> Self {
        match value {
            "RamAsc" => Self::RamAsc,
            "RamDesc" => Self::RamDesc,
            "VramAsc" => Self::VramAsc,
            "VramDesc" => Self::VramDesc,
            "NameAsc" => Self::NameAsc,
            "NameDesc" => Self::NameDesc,
            "PidAsc" => Self::PidAsc,
            "PidDesc" => Self::PidDesc,
            "AgeNewest" => Self::AgeNewest,
            "AgeOldest" => Self::AgeOldest,
            "RamGrowth" => Self::RamGrowth,
            "VramGrowth" => Self::VramGrowth,
            _ => Self::VramDesc,
        }
    }

    pub fn as_settings_value(self) -> &'static str {
        match self {
            Self::RamAsc => "RamAsc",
            Self::RamDesc => "RamDesc",
            Self::VramAsc => "VramAsc",
            Self::VramDesc => "VramDesc",
            Self::NameAsc => "NameAsc",
            Self::NameDesc => "NameDesc",
            Self::PidAsc => "PidAsc",
            Self::PidDesc => "PidDesc",
            Self::AgeNewest => "AgeNewest",
            Self::AgeOldest => "AgeOldest",
            Self::RamGrowth => "RamGrowth",
            Self::VramGrowth => "VramGrowth",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::RamAsc => "RAM low to high",
            Self::RamDesc => "RAM high to low",
            Self::VramAsc => "VRAM low to high",
            Self::VramDesc => "VRAM high to low",
            Self::NameAsc => "Name A to Z",
            Self::NameDesc => "Name Z to A",
            Self::PidAsc => "PID low to high",
            Self::PidDesc => "PID high to low",
            Self::AgeNewest => "Newest process",
            Self::AgeOldest => "Oldest process",
            Self::RamGrowth => "RAM growth",
            Self::VramGrowth => "VRAM growth",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProcessSnapshot {
    pub processes: Vec<ProcessInfo>,
    pub vram_status: String,
    pub listener_status: String,
    pub timing_status: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_web_table_prefers_common_ports_and_compacts_multiple_ports() {
        let process = ProcessInfo {
            local_endpoints: vec![
                ListeningEndpoint::new("127.0.0.1".to_string(), 9000, false),
                ListeningEndpoint::new("127.0.0.1".to_string(), 7860, false),
            ],
            ..Default::default()
        };

        assert_eq!(
            process.primary_local_web_url(),
            Some("http://127.0.0.1:7860")
        );
        assert_eq!(process.local_web_table_text(), "http://127.0.0.1:7860 (+1)");
    }

    #[test]
    fn local_web_table_shows_single_url() {
        let process = ProcessInfo {
            local_endpoints: vec![ListeningEndpoint::new("0.0.0.0".to_string(), 3000, false)],
            ..Default::default()
        };

        assert_eq!(process.local_web_table_text(), "http://127.0.0.1:3000");
    }

    #[test]
    fn local_web_table_shows_dash_when_absent() {
        let process = ProcessInfo::default();

        assert_eq!(process.local_web_table_text(), "-");
    }
}
