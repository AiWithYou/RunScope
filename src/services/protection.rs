use crate::settings::Settings;

const CRITICAL_PROCESS_NAMES: &[&str] = &[
    "System",
    "Registry",
    "Idle",
    "Memory Compression",
    "Secure System",
    "csrss.exe",
    "wininit.exe",
    "winlogon.exe",
    "services.exe",
    "lsass.exe",
    "smss.exe",
    "svchost.exe",
    "fontdrvhost.exe",
];

pub fn is_critical(name: &str) -> bool {
    CRITICAL_PROCESS_NAMES
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(name))
}

pub fn is_configured(name: &str, settings: &Settings) -> bool {
    settings
        .protected_process_names
        .iter()
        .any(|x| x.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn critical_processes_remain_protected_when_settings_list_is_empty() {
        let mut settings = Settings::default();
        settings.protected_process_names.clear();

        assert!(is_critical("LSASS.EXE"));
        assert!(!is_critical("notepad.exe"));
        assert!(!is_configured("notepad.exe", &settings));
    }
}
