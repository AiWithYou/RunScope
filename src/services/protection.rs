use crate::settings::Settings;

pub fn is_protected(name: &str, settings: &Settings) -> bool {
    settings
        .protected_process_names
        .iter()
        .any(|x| x.eq_ignore_ascii_case(name))
}
