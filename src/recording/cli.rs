use super::{report, worker, *};
use crate::{collectors::process_collector::ProcessCollector, settings::Settings};
use anyhow::{ensure, Context};
use std::path::PathBuf;
use std::time::Duration;

pub enum Command {
    Record {
        root: PathBuf,
        config: Config,
        pid: Option<u32>,
    },
    Report(PathBuf),
}

pub fn parse(args: &[String]) -> anyhow::Result<Command> {
    if args.first().map(String::as_str) == Some("--report") {
        ensure!(args.len() == 2, "Usage: --report <samples.jsonl>");
        return Ok(Command::Report(PathBuf::from(&args[1])));
    }
    ensure!(
        args.first().map(String::as_str) == Some("--record"),
        "expected --record"
    );
    let root = PathBuf::from(
        args.get(1)
            .filter(|s| !s.starts_with("--"))
            .context("--record requires an output directory")?,
    );
    let mut config = Config {
        duration_seconds: Some(60),
        ..Default::default()
    };
    let mut pid = None;
    let mut all = false;
    let mut seen = HashSet::new();
    let mut index = 2;
    while index < args.len() {
        let arg = args[index].as_str();
        ensure!(seen.insert(arg), "duplicate argument {arg}");
        match arg {
            "--all" => {
                all = true;
                config.mode = CaptureMode::All;
            }
            "--include-sensitive" => config.include_sensitive = true,
            "--pid" | "--duration" | "--interval" | "--max-log-mib" => {
                index += 1;
                let value = args
                    .get(index)
                    .with_context(|| format!("{arg} requires a value"))?;
                match arg {
                    "--pid" => {
                        let value: u32 = value.parse().context("invalid PID")?;
                        ensure!(value > 0, "PID must be positive");
                        pid = Some(value);
                    }
                    "--duration" => {
                        config.duration_seconds = Some(value.parse().context("invalid duration")?)
                    }
                    "--interval" => {
                        config.interval_seconds = value.parse().context("invalid interval")?
                    }
                    _ => {
                        config.max_log_bytes = value
                            .parse::<u64>()
                            .ok()
                            .and_then(|v| v.checked_mul(1024 * 1024))
                            .context("invalid log limit")?
                    }
                }
            }
            _ => bail!("unknown recording option {arg}"),
        }
        index += 1;
    }
    ensure!(
        !(all && pid.is_some()),
        "--all and --pid are mutually exclusive"
    );
    config.validate()?;
    Ok(Command::Record { root, config, pid })
}

pub fn execute(command: Command) -> anyhow::Result<()> {
    match command {
        Command::Report(path) => {
            let report = report::recover(&path)?;
            let directory = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| std::path::Path::new("."));
            // Never replace an input that happens to be named like a derived output.
            ensure!(
                !path
                    .file_name()
                    .is_some_and(|n| n.eq_ignore_ascii_case("report.json")
                        || n.eq_ignore_ascii_case("report.html")),
                "input must not be named report.json or report.html"
            );
            report::write_report(directory, &report)?;
            println!(
                "Recovered {} samples: {}",
                report.summary.sample_count,
                directory.join("report.html").display()
            );
        }
        Command::Record {
            root,
            mut config,
            pid,
        } => {
            let settings = Settings::load_or_default(&Settings::default_path())?;
            if let Some(pid) = pid {
                let snapshot = ProcessCollector::default().collect(&settings, false)?;
                config.mode = CaptureMode::Tree {
                    root: root_for_pid(&snapshot, pid)?,
                };
            }
            let mut run = worker::Run::start(&root, config, settings, None)?;
            println!("Recording to {}", run.directory.display());
            while run.active() {
                run.poll();
                std::thread::sleep(Duration::from_millis(100));
            }
            match run
                .outcome
                .take()
                .context("recorder returned no completion status")?
            {
                Ok(reason) => println!(
                    "Finished: {reason}\n{}",
                    run.directory.join("report.html").display()
                ),
                Err(error) => bail!("{error}"),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }
    #[test]
    fn rejects_ambiguous_and_out_of_range_commands() {
        for text in [
            "--record",
            "--record out --pid 0",
            "--record out --pid 10 --all",
            "--record out --interval 0",
            "--record out --interval 2 --interval 3",
            "--record out --duration 0",
            "--record out --unknown",
            "--report",
        ] {
            assert!(parse(&args(text)).is_err(), "accepted {text}");
        }
    }
    #[test]
    fn accepts_bounded_headless_recording() {
        let Command::Record { config, pid, .. } =
            parse(&args("--record out --duration 10 --interval 1 --pid 10")).unwrap()
        else {
            panic!("wrong command")
        };
        assert_eq!(pid, Some(10));
        assert_eq!(config.duration_seconds, Some(10));
        assert!(!config.include_sensitive);
    }
}
