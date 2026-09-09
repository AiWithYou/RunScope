//! Append-only JSONL is authoritative; JSON/HTML are bounded, replaceable summaries.
use super::*;
use crate::services::atomic_file::write_atomic;
use anyhow::{ensure, Context};
use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub header: Header,
    pub completion: String,
    pub log_bytes: u64,
    pub summary: Summary,
    pub last_observation: Vec<ProcessSample>,
}

pub struct Journal {
    file: File,
    pub directory: PathBuf,
    pub bytes: u64,
    limit: u64,
}

impl Journal {
    pub fn create(root: &Path, header: &Header) -> anyhow::Result<Self> {
        header.config.validate()?;
        std::fs::create_dir_all(root)
            .with_context(|| format!("cannot create recording root {}", root.display()))?;
        let mut directory = None;
        for suffix in 0..1000 {
            let candidate = root.join(format!(
                "session-{}-{}-{suffix}",
                header.started_unix_ms,
                std::process::id()
            ));
            match std::fs::create_dir(&candidate) {
                Ok(()) => {
                    directory = Some(candidate);
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
        let directory = directory.context("cannot allocate a unique session directory")?;
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(directory.join("samples.jsonl"))?;
        let mut journal = Self {
            file,
            directory,
            bytes: 0,
            limit: header.config.max_log_bytes,
        };
        ensure!(
            journal.append(&Record::Header(header.clone()))?,
            "log limit is too small for header"
        );
        Ok(journal)
    }

    /// Returns false before writing if the record would exceed the configured limit.
    pub fn append(&mut self, record: &Record) -> anyhow::Result<bool> {
        let mut bytes = serde_json::to_vec(record)?;
        ensure!(
            bytes.len() < MAX_LINE_BYTES,
            "sample exceeds the 4 MiB safety limit; narrow the recording scope"
        );
        bytes.push(b'\n');
        if self.bytes.saturating_add(bytes.len() as u64) > self.limit {
            return Ok(false);
        }
        self.file.write_all(&bytes)?;
        self.file.flush()?;
        self.bytes += bytes.len() as u64;
        Ok(true)
    }

    pub fn sync(&self) -> anyhow::Result<()> {
        self.file
            .sync_all()
            .context("cannot sync recording journal")
    }
}

pub fn write_report(directory: &Path, report: &Report) -> anyhow::Result<()> {
    write_atomic(
        &directory.join("report.json"),
        &serde_json::to_vec_pretty(report)?,
    )?;
    write_atomic(&directory.join("report.html"), html(report).as_bytes())?;
    Ok(())
}

pub fn recover(path: &Path) -> anyhow::Result<Report> {
    let file = File::open(path).with_context(|| format!("cannot read {}", path.display()))?;
    let log_bytes = file.metadata()?.len();
    ensure!(
        log_bytes <= 4 * 1024 * 1024 * 1024,
        "journal exceeds 4 GiB safety limit"
    );
    let mut reader = BufReader::new(file);
    let mut header: Option<Header> = None;
    let mut summary = Summary::default();
    let mut last_observation = Vec::new();
    let mut completion = None;
    let mut previous_ms = 0;
    let mut truncated = false;
    loop {
        let mut line = Vec::new();
        let read = reader
            .by_ref()
            .take((MAX_LINE_BYTES + 1) as u64)
            .read_until(b'\n', &mut line)?;
        if read == 0 {
            break;
        }
        ensure!(
            read <= MAX_LINE_BYTES,
            "journal line exceeds 4 MiB safety limit"
        );
        ensure!(
            completion.is_none(),
            "journal contains data after its end record"
        );
        let record = match serde_json::from_slice::<Record>(&line) {
            Ok(record) => record,
            Err(_)
                if !line.ends_with(b"\n") && reader.fill_buf()?.is_empty() && header.is_some() =>
            {
                truncated = true;
                break;
            }
            Err(error) => {
                return Err(error).context(
                    "invalid journal record (only a truncated final line can be recovered)",
                )
            }
        };
        match record {
            Record::Header(value) => {
                ensure!(header.is_none(), "duplicate journal header");
                ensure!(
                    value.schema_version == SCHEMA_VERSION,
                    "unsupported journal schema {}",
                    value.schema_version
                );
                value.config.validate()?;
                header = Some(value);
            }
            Record::Frame(frame) => {
                let header = header.as_ref().context("frame precedes journal header")?;
                ensure!(
                    frame.sequence == summary.sample_count,
                    "non-contiguous frame sequence"
                );
                ensure!(frame.elapsed_ms >= previous_ms, "non-monotonic sample time");
                previous_ms = frame.elapsed_ms;
                summary.push(&frame, header.config.interval_seconds * 1000);
                last_observation = frame.processes;
            }
            Record::End { reason } => {
                ensure!(header.is_some(), "end precedes journal header");
                completion = Some(reason);
            }
        }
    }
    Ok(Report {
        header: header.context("journal has no complete header")?,
        summary,
        last_observation,
        log_bytes,
        completion: completion.unwrap_or_else(|| {
            if truncated {
                "recovered: incomplete final record discarded".into()
            } else {
                "recovered: recording has no end marker (interrupted or still active)".into()
            }
        }),
    })
}

pub fn html(report: &Report) -> String {
    let mut out = String::from(
        r#"<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src data:"><title>RunScope flight report</title><style>body{font:15px system-ui,sans-serif;background:#0f1520;color:#dde5f2;margin:32px auto;padding:0 24px;max-width:1200px}h1{font-size:32px}h2{margin-top:32px}small,.muted{color:#a5b4ca}.cards{display:flex;gap:20px;flex-wrap:wrap}.card{background:#1b2638;padding:18px;border-radius:10px;min-width:170px}.card b{display:block;font-size:24px;margin-top:8px}table{border-collapse:collapse;width:100%;font-size:13px}td,th{border-bottom:1px solid #344158;text-align:left;padding:9px;overflow-wrap:anywhere}svg{width:100%;height:auto;background:#1b2638;border-radius:10px}code{overflow-wrap:anywhere}.scroll{overflow-x:auto}.note{border-left:3px solid #f0bd68;padding:12px 18px;background:#1b2638}pre{white-space:pre-wrap;overflow-wrap:anywhere}</style><h1>RunScope flight report</h1>"#,
    );
    let s = &report.summary;
    let _ = write!(
        out,
        "<p>Version {} · schema {} · start (Unix ms): {}<br>Status: <strong>{}</strong></p>",
        escape(&report.header.app_version),
        report.header.schema_version,
        report.header.started_unix_ms,
        escape(&report.completion)
    );
    let _ = write!(out, "<div class=cards><div class=card>Recorded samples<b>{}</b></div><div class=card>Peak working-set sum<b>{:.1} MiB</b></div><div class=card>Peak known VRAM<b>{}</b></div><div class=card>Collection over interval<b>{}</b></div></div>",
        s.sample_count, mib(s.peak_ram_bytes), optional_mib(s.peak_vram_bytes_known), s.slow_sample_count);
    out.push_str("<p class=note>Observations, not a crash diagnosis. A missing process may have exited normally, become inaccessible, or fallen outside the scope. No exit codes or GPU utilization are inferred. VRAM totals include only known values; working-set sums may double-count shared pages. CPU 100% represents one logical processor. Process I/O is not necessarily physical disk traffic.</p>");
    let _ = write!(out, "<p class=muted>Preview: last {} points ({} older points omitted); last {} events ({} older events omitted). All complete samples and events remain in samples.jsonl. Sampling can miss activity between observations. Byte sizes use binary MiB.</p>",
        s.points.len(), s.omitted_preview_points, s.events.len(), s.omitted_events);
    out.push_str("<h2>Working-set sum · MiB</h2>");
    out.push_str(&chart(s, false));
    out.push_str("<h2>Known VRAM sum · MiB</h2>");
    out.push_str(&chart(s, true));
    out.push_str("<h2>Last observed processes</h2><div class=scroll><table><tr><th>PID / name</th><th>Workload hint</th><th>RAM MiB</th><th>VRAM MiB</th><th>CPU %</th><th>I/O read / write delta bytes</th></tr>");
    for p in &report.last_observation {
        let _ = write!(out, "<tr><td>{} · {}</td><td>{}</td><td>{:.1}</td><td>{}</td><td>{}</td><td>{} / {}</td></tr>", p.pid,
            escape(&p.name), escape(&p.workload_hint), mib(p.ram_bytes), optional_mib(p.vram_bytes),
            p.cpu_percent.map(|v| format!("{v:.1}")).unwrap_or_else(|| "N/A".into()),
            p.io_read_bytes_delta.map(|v| v.to_string()).unwrap_or_else(|| "N/A".into()),
            p.io_write_bytes_delta.map(|v| v.to_string()).unwrap_or_else(|| "N/A".into()));
    }
    out.push_str("</table></div><h2>Observation events</h2><table><tr><th>Elapsed s</th><th>Event</th><th>Process / last RAM</th><th>Meaning</th></tr>");
    for e in s.events.iter().rev() {
        let process = e
            .process
            .as_ref()
            .map(|p| format!("{} · {} · {:.1} MiB", p.pid, p.name, mib(p.ram_bytes)))
            .unwrap_or_default();
        let _ = write!(
            out,
            "<tr><td>{:.1}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            e.elapsed_ms as f64 / 1000.0,
            escape(&e.kind),
            escape(&process),
            escape(&e.message)
        );
    }
    out.push_str("</table><h2>Recording configuration</h2><pre>");
    out.push_str(&escape(
        &serde_json::to_string_pretty(&report.header.config).unwrap_or_default(),
    ));
    out.push_str("</pre><p class=muted>No external assets, scripts, telemetry or network requests. Review process names, PIDs and optional sensitive fields before sharing the JSONL/JSON files.</p></html>");
    out
}

fn chart(summary: &Summary, vram: bool) -> String {
    let value = |p: &Point| {
        if vram {
            p.totals.vram_bytes_known
        } else {
            Some(p.totals.ram_bytes)
        }
    };
    let max = summary
        .points
        .iter()
        .filter_map(value)
        .max()
        .unwrap_or(0)
        .max(1) as f64;
    let min_time = summary.points.front().map_or(0, |p| p.elapsed_ms);
    let max_time = summary.points.back().map_or(min_time, |p| p.elapsed_ms);
    let span = max_time.saturating_sub(min_time).max(1) as f64;
    let mut path = String::new();
    let mut gap = true;
    let mut circles = String::new();
    for p in &summary.points {
        if let Some(v) = value(p) {
            let x = 60.0 + p.elapsed_ms.saturating_sub(min_time) as f64 / span * 890.0;
            let y = 175.0 - v as f64 / max * 145.0;
            let _ = write!(path, "{} {x:.2} {y:.2} ", if gap { "M" } else { "L" });
            let _ = write!(
                circles,
                "<circle cx='{x:.2}' cy='{y:.2}' r='2' fill='#7dcfff'/>"
            );
            gap = false;
        } else {
            gap = true;
        }
    }
    format!("<svg viewBox='0 0 980 220' role='img' aria-label='Memory timeline; gaps mean unavailable'><path d='{path}' fill='none' stroke='#7dcfff' stroke-width='2'/>{circles}<g fill='#b3c2d8' font-size='12'><text x='8' y='25'>{:.1}</text><text x='15' y='180'>0</text><text x='60' y='205'>{:.1} s</text><text x='860' y='205'>{:.1} s</text></g></svg>", max / 1048576.0, min_time as f64 / 1000.0, max_time as f64 / 1000.0)
}

pub fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(c),
        }
    }
    escaped
}
fn mib(value: u64) -> f64 {
    value as f64 / 1048576.0
}
fn optional_mib(value: Option<u64>) -> String {
    value
        .map(|v| format!("{:.1}", mib(v)))
        .unwrap_or_else(|| "N/A".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "runscope-report-test-{}-{}-{}",
            std::process::id(),
            unix_ms(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
    #[test]
    fn recovery_keeps_complete_prefix_and_rejects_corrupt_middle() {
        let root = root();
        let header = Header::new(Config::default());
        let mut journal = Journal::create(&root, &header).unwrap();
        let frame = Tracker::new(Config::default()).sample(ProcessSnapshot::default(), 0, 0);
        journal.append(&Record::Frame(Box::new(frame))).unwrap();
        let path = journal.directory.join("samples.jsonl");
        drop(journal);
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{\"record\":")
            .unwrap();
        let report = recover(&path).unwrap();
        assert_eq!(report.summary.sample_count, 1);
        assert!(report.completion.contains("incomplete"));
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"\n")
            .unwrap();
        assert!(recover(&path).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn completed_report_round_trips_and_html_is_escaped() {
        let root = root();
        let header = Header::new(Config::default());
        let mut journal = Journal::create(&root, &header).unwrap();
        journal
            .append(&Record::End {
                reason: "<script>alert('x')</script>".into(),
            })
            .unwrap();
        let report = recover(&journal.directory.join("samples.jsonl")).unwrap();
        let text = html(&report);
        assert!(!text.contains("<script>"));
        assert!(text.contains("&lt;script&gt;"));
        write_report(&journal.directory, &report).unwrap();
        assert!(journal.directory.join("report.html").exists());
        drop(journal);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn unique_directories_do_not_overwrite_previous_recordings() {
        let root = root();
        let header = Header::new(Config::default());
        let a = Journal::create(&root, &header).unwrap();
        let b = Journal::create(&root, &header).unwrap();
        assert_ne!(a.directory, b.directory);
        drop((a, b));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn unsupported_schema_and_duplicate_headers_are_rejected() {
        let root = root();
        let mut header = Header::new(Config::default());
        header.schema_version = 99;
        let journal = Journal::create(&root, &header).unwrap();
        assert!(recover(&journal.directory.join("samples.jsonl")).is_err());
        drop(journal);
        std::fs::remove_dir_all(root).unwrap();
    }
}
