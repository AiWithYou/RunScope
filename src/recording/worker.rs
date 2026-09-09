use super::{
    report::{write_report, Journal, Report},
    *,
};
use crate::{collectors::process_collector::ProcessCollector, settings::Settings};
use anyhow::Context;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default)]
pub struct Progress {
    pub samples: u64,
    pub elapsed_ms: u64,
    pub collection_ms: u64,
    pub log_bytes: u64,
    pub totals: Totals,
    pub last_events: Vec<Event>,
    pub points: Vec<Point>,
    pub vram_status: String,
}

pub struct Run {
    pub directory: PathBuf,
    pub progress: Progress,
    pub stopping: bool,
    pub outcome: Option<Result<String, String>>,
    receiver: mpsc::Receiver<Progress>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<anyhow::Result<String>>>,
}

impl Run {
    pub fn start(
        root: &Path,
        config: Config,
        settings: Settings,
        repaint: Option<egui::Context>,
    ) -> anyhow::Result<Self> {
        config.validate()?;
        let header = Header::new(config);
        let journal = Journal::create(root, &header)?;
        let directory = journal.directory.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let handle = thread::Builder::new()
            .name("runscope-recorder".into())
            .spawn(move || {
                let result = record(
                    journal,
                    header,
                    settings,
                    &worker_stop,
                    sender,
                    repaint.as_ref(),
                );
                if let Some(ctx) = repaint {
                    ctx.request_repaint();
                }
                result
            })
            .context("cannot start recorder worker")?;
        Ok(Self {
            directory,
            progress: Progress::default(),
            stopping: false,
            outcome: None,
            receiver,
            stop,
            handle: Some(handle),
        })
    }

    pub fn stop(&mut self) {
        self.stopping = true;
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = &self.handle {
            handle.thread().unpark();
        }
    }

    pub fn poll(&mut self) {
        while let Ok(progress) = self.receiver.try_recv() {
            self.progress = progress;
        }
        if self.handle.as_ref().is_some_and(|h| h.is_finished()) {
            let handle = self.handle.take().expect("checked recorder handle");
            self.outcome = Some(match handle.join() {
                Ok(result) => result.map_err(|e| format!("{e:#}")),
                Err(_) => {
                    Err("recorder worker panicked; recover samples.jsonl with --report".into())
                }
            });
        }
    }

    pub fn active(&self) -> bool {
        self.handle.is_some()
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        self.stop();
    }
}

fn record(
    mut journal: Journal,
    header: Header,
    settings: Settings,
    stop: &AtomicBool,
    sender: mpsc::SyncSender<Progress>,
    repaint: Option<&egui::Context>,
) -> anyhow::Result<String> {
    let mut collector = ProcessCollector::default();
    let mut tracker = Tracker::new(header.config.clone());
    let mut summary = Summary::default();
    let mut last_observation = Vec::new();
    let started = Instant::now();
    let interval = Duration::from_secs(header.config.interval_seconds);
    let duration = header.config.duration_seconds.map(Duration::from_secs);
    let result = (|| -> anyhow::Result<String> {
        loop {
            if stop.load(Ordering::Relaxed) {
                return Ok("stopped by user".into());
            }
            if duration.is_some_and(|d| started.elapsed() >= d) {
                return Ok("duration reached".into());
            }
            let collecting = Instant::now();
            let snapshot = collector.collect(&settings, true)?;
            let collection_ms = collecting.elapsed().as_millis().min(u64::MAX as u128) as u64;
            let frame = tracker.sample(
                snapshot,
                started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                collection_ms,
            );
            let record = Record::Frame(Box::new(frame));
            if !journal.append(&record)? {
                return Ok("log size limit reached; final sample was not written".into());
            }
            let Record::Frame(frame) = record else {
                unreachable!()
            };
            summary.push(&frame, header.config.interval_seconds * 1000);
            last_observation = frame.processes;
            let _ = sender.try_send(Progress {
                samples: summary.sample_count,
                elapsed_ms: frame.elapsed_ms,
                collection_ms,
                log_bytes: journal.bytes,
                totals: frame.totals,
                last_events: summary.events.iter().rev().take(8).cloned().collect(),
                points: summary
                    .points
                    .iter()
                    .rev()
                    .take(120)
                    .cloned()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect(),
                vram_status: frame.vram_status,
            });
            if let Some(ctx) = repaint {
                ctx.request_repaint();
            }
            // No catch-up burst. Stop wakes this wait; overruns get a minimum quiet period.
            let remaining = interval
                .saturating_sub(collecting.elapsed())
                .max(Duration::from_millis(100));
            let remaining = duration.map_or(remaining, |d| {
                remaining.min(d.saturating_sub(started.elapsed()))
            });
            let wake_at = Instant::now() + remaining;
            while !stop.load(Ordering::Relaxed) && Instant::now() < wake_at {
                thread::park_timeout(wake_at.saturating_duration_since(Instant::now()));
            }
        }
    })();
    let reason = match &result {
        Ok(reason) => reason.clone(),
        Err(error) => format!("recording error: {error:#}"),
    };
    // Reserve no in-memory backlog. If the journal is full, derived reports still explain the stop.
    let end_result = if result.is_ok() {
        journal.append(&Record::End {
            reason: reason.clone(),
        })
    } else {
        // An I/O error may have left a partial final line. Leave it recoverable.
        Ok(false)
    };
    let sync_result = journal.sync();
    let report = Report {
        header,
        completion: reason.clone(),
        log_bytes: journal.bytes,
        summary,
        last_observation,
    };
    let export_result = write_report(&journal.directory, &report);
    end_result.context("could not finish journal; try --report on its complete prefix")?;
    sync_result?;
    export_result.context("journal preserved, but report export failed; try --report")?;
    result.map(|_| reason)
}

pub fn default_root() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("RunScope")
        .join("recordings")
}
