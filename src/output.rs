// Copyright (c) 2026 Andrea Bodei <info@andreabodei.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Headless snapshot and newline-delimited streaming output.

use std::{
    io::{self, Write},
    time::{Duration, Instant},
};

use anyhow::Result;

use crate::{
    app::{process_matches, AppState},
    cli::Cli,
    metrics::{Collector, Snapshot},
};

pub fn run(cli: &Cli) -> Result<()> {
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    let result = if cli.json_stream {
        stream_to(cli, &mut writer)
    } else {
        let snapshot = collect_snapshot(cli)?;
        write_snapshot(&mut writer, &snapshot, true).map_err(Into::into)
    };
    // Pipelines such as `ztop --json-stream | head -n 1` are normal exits.
    match result {
        Err(error)
            if error
                .downcast_ref::<io::Error>()
                .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe) =>
        {
            Ok(())
        }
        other => other,
    }
}

fn warmed_collector(cli: &Cli) -> Result<Collector> {
    let mut collector = Collector::new(cli)?;
    collector.sample_gpu();
    // CPU usage is a delta between readings. GPU observations should also be
    // refreshed after warmup so the first output does not carry an old sample.
    std::thread::sleep(Duration::from_millis(250));
    collector.sample_gpu_if_due(Instant::now());
    Ok(collector)
}

fn select_processes(snapshot: &mut Snapshot, app: &AppState) {
    let query = app.process_filter.to_lowercase();
    snapshot.procs.retain(|row| process_matches(row, &query));
    snapshot.procs.truncate(app.top_n);
}

fn collect_snapshot(cli: &Cli) -> Result<Snapshot> {
    let app = AppState::from_cli(cli);
    let mut snapshot = warmed_collector(cli)?.collect(&app);
    select_processes(&mut snapshot, &app);
    Ok(snapshot)
}

fn stream_to(cli: &Cli, writer: &mut impl Write) -> Result<()> {
    let app = AppState::from_cli(cli);
    let mut collector = warmed_collector(cli)?;
    let mut emitted = 0u64;
    loop {
        let next_sample = Instant::now() + app.refresh_interval();
        let mut snapshot = collector.collect(&app);
        select_processes(&mut snapshot, &app);
        write_snapshot(writer, &snapshot, false)?;
        emitted += 1;
        if cli.count.is_some_and(|count| emitted >= count) {
            return Ok(());
        }
        // Device sampling keeps its 250 ms cadence between outputs. A blocked
        // downstream writer applies backpressure to collection as well.
        loop {
            let now = Instant::now();
            collector.sample_gpu_if_due(now);
            let now = Instant::now();
            if now >= next_sample {
                break;
            }
            let remaining = next_sample.duration_since(now);
            let wait = collector
                .gpu_sample_wait(now)
                .map(|gpu_wait| gpu_wait.min(remaining))
                .unwrap_or(remaining);
            std::thread::sleep(wait);
        }
    }
}

fn write_snapshot(writer: &mut impl Write, snapshot: &Snapshot, pretty: bool) -> io::Result<()> {
    let bytes = if pretty {
        serde_json::to_vec_pretty(snapshot)
    } else {
        serde_json::to_vec(snapshot)
    }
    .map_err(io::Error::other)?;
    writer.write_all(&bytes)?;
    writer.write_all(b"\n")?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::SNAPSHOT_SCHEMA_VERSION;
    use clap::Parser;

    #[test]
    fn snapshot_honors_top_and_disable_flags() {
        let cli = Cli::parse_from(["ztop", "--json", "--top", "1", "--no-gpu", "--no-temp"]);
        let snapshot = collect_snapshot(&cli).unwrap();
        assert_eq!(snapshot.schema_version, SNAPSHOT_SCHEMA_VERSION);
        assert!(snapshot.sampled_at_unix_ms > 0);
        assert!(snapshot.procs.len() <= 1);
        assert!(snapshot.gpus.is_empty());
        assert!(snapshot.cpu_temp_c.is_none());
        assert!(snapshot.disks.iter().all(|disk| disk.temp_c.is_none()));
    }

    #[test]
    fn stream_emits_independent_json_lines_and_applies_filter() {
        let cli = Cli::parse_from([
            "ztop",
            "--json-stream",
            "--count",
            "2",
            "--refresh",
            "0.25",
            "--no-gpu",
            "--no-temp",
            "--filter",
            "ztop-deliberately-absent-process-7ab916",
        ]);
        let mut output = Vec::new();
        stream_to(&cli, &mut output).unwrap();
        let text = String::from_utf8(output).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        let samples: Vec<serde_json::Value> = lines
            .iter()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        for sample in &samples {
            assert_eq!(sample["schema_version"], SNAPSHOT_SCHEMA_VERSION);
            assert_eq!(sample["procs"].as_array().unwrap().len(), 0);
        }
        assert!(
            samples[1]["sampled_at_unix_ms"].as_u64().unwrap()
                > samples[0]["sampled_at_unix_ms"].as_u64().unwrap()
        );
    }

    #[test]
    fn stream_stops_on_broken_pipe() {
        struct ClosedPipe;
        impl Write for ClosedPipe {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::from(io::ErrorKind::BrokenPipe))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let cli = Cli::parse_from(["ztop", "--json-stream", "--no-gpu", "--no-temp"]);
        let error = stream_to(&cli, &mut ClosedPipe).unwrap_err();
        assert_eq!(
            error.downcast_ref::<io::Error>().unwrap().kind(),
            io::ErrorKind::BrokenPipe
        );
    }
}
