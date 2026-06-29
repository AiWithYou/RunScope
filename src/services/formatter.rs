pub fn bytes_to_mb_text(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1024.0 / 1024.0)
}

pub fn optional_bytes_to_mb_text(bytes: Option<u64>) -> String {
    match bytes {
        Some(value) => bytes_to_mb_text(value),
        None => "N/A".to_string(),
    }
}

pub fn age_text(start_time: Option<std::time::SystemTime>) -> String {
    let Some(start_time) = start_time else {
        return String::new();
    };
    let Ok(elapsed) = std::time::SystemTime::now().duration_since(start_time) else {
        return String::new();
    };

    let total_seconds = elapsed.as_secs();
    let days = total_seconds / 86_400;
    let hours = (total_seconds % 86_400) / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;

    if days > 0 {
        format!("{days}d {hours:02}:{minutes:02}")
    } else {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    }
}
