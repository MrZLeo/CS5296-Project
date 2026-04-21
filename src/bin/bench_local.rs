use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use my_app::{parse_app_output_json, Workload, DEFAULT_ALLOC_BYTES, DEFAULT_BENCH_FIB_N};

const DEFAULT_SAMPLES: usize = 30;
const DEFAULT_DOCKER_IMAGE: &str = "my-docker-app:latest";
const DEFAULT_WASM_ARTIFACT: &str = "target/wasm32-wasip1/release/my-app.wasm";
const DEFAULT_WASM_AOT_ARTIFACT: &str = "my-app-aot.wasm";
const DEFAULT_OUTPUT_PATH: &str = "target/bench-results/local-bench.csv";
const DEFAULT_SUMMARY_OUTPUT_PATH: &str = "target/bench-results/summary.csv";

#[derive(Clone, Debug)]
struct BenchConfig {
    samples: usize,
    docker_image: String,
    wasm_artifact: PathBuf,
    wasm_aot_artifact: PathBuf,
    output_path: PathBuf,
    summary_output_path: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Runtime {
    WasmEdgeWasm,
    WasmEdgeAot,
    Docker,
}

#[derive(Clone, Debug)]
struct SampleRecord {
    runtime: &'static str,
    workload: String,
    parameter: String,
    sample: usize,
    e2e_ms: f64,
    internal_compute_ms: f64,
    startup_overhead_ms: f64,
    exit_code: i32,
}

#[derive(Clone, Debug, PartialEq)]
struct SummaryRecord {
    runtime: &'static str,
    workload: String,
    parameter: String,
    samples: usize,
    mean: f64,
    p50: f64,
    p95: f64,
    stddev: f64,
    startup_mean: f64,
}

fn main() {
    let config = match parse_bench_args(std::env::args().skip(1)) {
        Ok(config) => config,
        Err(error) if error == "--help" => {
            println!("{}", bench_usage());
            return;
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };

    if let Err(error) = run_benchmark(&config) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run_benchmark(config: &BenchConfig) -> Result<(), String> {
    for runtime in Runtime::all() {
        runtime.preflight(config)?;
    }

    let workloads = vec![
        Workload::Noop,
        Workload::Fib {
            n: DEFAULT_BENCH_FIB_N,
        },
        Workload::AllocTouch {
            bytes: DEFAULT_ALLOC_BYTES,
            hold_ms: 0,
        },
    ];

    let mut records = Vec::with_capacity(config.samples * workloads.len() * 2);
    for runtime in Runtime::all() {
        for workload in &workloads {
            for sample in 1..=config.samples {
                let record = runtime.run_sample(workload, sample, config)?;
                println!(
                    "[{}/{}] {} {} {} e2e={:.3}ms internal={:.3}ms startup={:.3}ms",
                    sample,
                    config.samples,
                    runtime.name(),
                    workload.name(),
                    workload.parameter(),
                    record.e2e_ms,
                    record.internal_compute_ms,
                    record.startup_overhead_ms,
                );
                records.push(record);
            }
        }
    }

    write_csv(config.output_path.as_path(), &records)?;
    let summary = build_summary(&records);
    write_summary_csv(config.summary_output_path.as_path(), &summary)?;
    print_summary(&summary);
    println!("\nDetail CSV written to {}", config.output_path.display());
    println!(
        "Summary CSV written to {}",
        config.summary_output_path.display()
    );
    Ok(())
}

fn parse_bench_args<I, S>(args: I) -> Result<BenchConfig, String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<String>>();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return Err("--help".to_string());
    }

    let mut samples = DEFAULT_SAMPLES;
    let mut docker_image = DEFAULT_DOCKER_IMAGE.to_string();
    let mut wasm_artifact = PathBuf::from(DEFAULT_WASM_ARTIFACT);
    let mut wasm_aot_artifact = PathBuf::from(DEFAULT_WASM_AOT_ARTIFACT);
    let mut output_path = PathBuf::from(DEFAULT_OUTPUT_PATH);
    let mut summary_output_path = PathBuf::from(DEFAULT_SUMMARY_OUTPUT_PATH);

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--samples" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "missing value for --samples".to_string())?;
                samples = value
                    .parse::<usize>()
                    .map_err(|_| format!("invalid value for --samples: {value}"))?;
                if samples == 0 {
                    return Err("--samples must be at least 1".to_string());
                }
                index += 2;
            }
            "--docker-image" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "missing value for --docker-image".to_string())?;
                docker_image = value.clone();
                index += 2;
            }
            "--wasm-artifact" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "missing value for --wasm-artifact".to_string())?;
                wasm_artifact = PathBuf::from(value);
                index += 2;
            }
            "--wasm-aot-artifact" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "missing value for --wasm-aot-artifact".to_string())?;
                wasm_aot_artifact = PathBuf::from(value);
                index += 2;
            }
            "--output" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "missing value for --output".to_string())?;
                output_path = PathBuf::from(value);
                index += 2;
            }
            "--summary-output" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "missing value for --summary-output".to_string())?;
                summary_output_path = PathBuf::from(value);
                index += 2;
            }
            unknown => {
                return Err(format!("unknown argument: {unknown}\n\n{}", bench_usage()));
            }
        }
    }

    Ok(BenchConfig {
        samples,
        docker_image,
        wasm_artifact,
        wasm_aot_artifact,
        output_path,
        summary_output_path,
    })
}

fn bench_usage() -> &'static str {
    "\
Usage:
  cargo run --bin bench_local --release -- [options]

Options:
  --samples <usize>         Number of sequential cold-start samples per workload (default: 30)
  --docker-image <tag>      Docker image tag to run (default: my-docker-app:latest)
  --wasm-artifact <path>    Plain Wasm artifact path for WasmEdge (default: target/wasm32-wasip1/release/my-app.wasm)
  --wasm-aot-artifact <path> AOT WasmEdge artifact path (default: my-app-aot.wasm)
  --output <path>           Detail CSV path (default: target/bench-results/local-bench.csv)
  --summary-output <path>   Summary CSV path (default: target/bench-results/summary.csv)"
}

impl Runtime {
    fn all() -> [Self; 3] {
        [Self::WasmEdgeWasm, Self::WasmEdgeAot, Self::Docker]
    }

    fn name(self) -> &'static str {
        match self {
            Self::WasmEdgeWasm => "wasmedge-wasm",
            Self::WasmEdgeAot => "wasmedge-aot",
            Self::Docker => "docker",
        }
    }

    fn preflight(self, config: &BenchConfig) -> Result<(), String> {
        match self {
            Self::WasmEdgeWasm => {
                if !config.wasm_artifact.is_file() {
                    return Err(format!(
                        "missing plain Wasm artifact: {}",
                        config.wasm_artifact.display()
                    ));
                }
                check_command_success("wasmedge", &["--version"], None)
                    .map_err(|error| format!("WasmEdge preflight failed: {error}"))?;
            }
            Self::WasmEdgeAot => {
                if !config.wasm_aot_artifact.is_file() {
                    return Err(format!(
                        "missing AOT Wasm artifact: {}",
                        config.wasm_aot_artifact.display()
                    ));
                }
                check_command_success("wasmedge", &["--version"], None)
                    .map_err(|error| format!("WasmEdge preflight failed: {error}"))?;
            }
            Self::Docker => {
                check_command_success(
                    "docker",
                    &["image", "inspect", config.docker_image.as_str()],
                    Some(format!(
                        "Build it with: docker build -t {} .",
                        config.docker_image
                    )),
                )
                .map_err(|error| format!("Docker preflight failed: {error}"))?;
            }
        }
        Ok(())
    }

    fn run_sample(
        self,
        workload: &Workload,
        sample: usize,
        config: &BenchConfig,
    ) -> Result<SampleRecord, String> {
        let args = workload.cli_args();
        let mut command = match self {
            Self::WasmEdgeWasm => {
                let mut command = Command::new("wasmedge");
                command.arg(&config.wasm_artifact);
                command.args(&args);
                command
            }
            Self::WasmEdgeAot => {
                let mut command = Command::new("wasmedge");
                command.arg(&config.wasm_aot_artifact);
                command.args(&args);
                command
            }
            Self::Docker => {
                let mut command = Command::new("docker");
                command.arg("run").arg("--rm").arg(&config.docker_image);
                command.args(&args);
                command
            }
        };

        let started = Instant::now();
        let output = command.output().map_err(|error| {
            format!("failed to launch {} sample {sample}: {error}", self.name())
        })?;
        let e2e_ms = started.elapsed().as_secs_f64() * 1000.0;
        let exit_code = output.status.code().unwrap_or(-1);

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "{} {} sample {} failed with exit code {}: {}",
                self.name(),
                workload.name(),
                sample,
                exit_code,
                stderr.trim()
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json_line = stdout
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .ok_or_else(|| {
                format!(
                    "{} {} sample {} produced no stdout",
                    self.name(),
                    workload.name(),
                    sample
                )
            })?;
        let app_output = parse_app_output_json(json_line.trim()).map_err(|error| {
            format!(
                "{} {} sample {} produced invalid JSON output: {}",
                self.name(),
                workload.name(),
                sample,
                error
            )
        })?;

        Ok(SampleRecord {
            runtime: self.name(),
            workload: app_output.workload,
            parameter: app_output.parameter,
            sample,
            e2e_ms,
            internal_compute_ms: app_output.internal_compute_ms,
            startup_overhead_ms: (e2e_ms - app_output.internal_compute_ms).max(0.0),
            exit_code,
        })
    }
}

fn check_command_success(program: &str, args: &[&str], hint: Option<String>) -> Result<(), String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("failed to run `{program}`: {error}"))?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut message = format!(
        "`{} {}` exited with {}",
        program,
        args.join(" "),
        output.status.code().unwrap_or(-1)
    );
    if !stderr.trim().is_empty() {
        message.push_str(": ");
        message.push_str(stderr.trim());
    }
    if let Some(hint) = hint {
        message.push('\n');
        message.push_str(&hint);
    }
    Err(message)
}

fn create_parent_dir(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create benchmark output directory {}: {error}",
                parent.display()
            )
        })?;
    }
    Ok(())
}

fn write_csv(path: &Path, records: &[SampleRecord]) -> Result<(), String> {
    create_parent_dir(path)?;

    let file = File::create(path)
        .map_err(|error| format!("failed to create benchmark CSV {}: {error}", path.display()))?;
    let mut writer = BufWriter::new(file);
    writeln!(
        writer,
        "runtime,workload,parameter,sample,e2e_ms,internal_compute_ms,startup_overhead_ms,exit_code"
    )
    .map_err(|error| format!("failed to write CSV header: {error}"))?;

    for record in records {
        writeln!(
            writer,
            "{},{},{},{},{:.6},{:.6},{:.6},{}",
            record.runtime,
            record.workload,
            record.parameter,
            record.sample,
            record.e2e_ms,
            record.internal_compute_ms,
            record.startup_overhead_ms,
            record.exit_code,
        )
        .map_err(|error| format!("failed to write CSV row: {error}"))?;
    }

    writer
        .flush()
        .map_err(|error| format!("failed to flush CSV writer: {error}"))?;
    Ok(())
}

fn build_summary(records: &[SampleRecord]) -> Vec<SummaryRecord> {
    let mut summary = Vec::new();
    for runtime in Runtime::all() {
        for workload in [
            Workload::Noop,
            Workload::Fib {
                n: DEFAULT_BENCH_FIB_N,
            },
            Workload::AllocTouch {
                bytes: DEFAULT_ALLOC_BYTES,
                hold_ms: 0,
            },
        ] {
            let parameter = workload.parameter();
            let group = records
                .iter()
                .filter(|record| {
                    record.runtime == runtime.name()
                        && record.workload == workload.name()
                        && record.parameter == parameter
                })
                .collect::<Vec<_>>();
            if group.is_empty() {
                continue;
            }

            let e2e_values = group.iter().map(|record| record.e2e_ms).collect::<Vec<_>>();
            let startup_values = group
                .iter()
                .map(|record| record.startup_overhead_ms)
                .collect::<Vec<_>>();
            let stats = Stats::from_values(&e2e_values);

            summary.push(SummaryRecord {
                runtime: runtime.name(),
                workload: workload.name().to_string(),
                parameter,
                samples: group.len(),
                mean: stats.mean,
                p50: stats.p50,
                p95: stats.p95,
                stddev: stats.stddev,
                startup_mean: mean(&startup_values),
            });
        }
    }
    summary
}

fn write_summary_csv(path: &Path, summary: &[SummaryRecord]) -> Result<(), String> {
    create_parent_dir(path)?;

    let file = File::create(path)
        .map_err(|error| format!("failed to create summary CSV {}: {error}", path.display()))?;
    let mut writer = BufWriter::new(file);
    writeln!(
        writer,
        "runtime,workload,parameter,samples,mean,p50,p95,stddev,startup_mean"
    )
    .map_err(|error| format!("failed to write summary CSV header: {error}"))?;

    for row in summary {
        writeln!(
            writer,
            "{},{},{},{},{:.6},{:.6},{:.6},{:.6},{:.6}",
            row.runtime,
            row.workload,
            row.parameter,
            row.samples,
            row.mean,
            row.p50,
            row.p95,
            row.stddev,
            row.startup_mean,
        )
        .map_err(|error| format!("failed to write summary CSV row: {error}"))?;
    }

    writer
        .flush()
        .map_err(|error| format!("failed to flush summary CSV writer: {error}"))?;
    Ok(())
}

fn print_summary(summary: &[SummaryRecord]) {
    println!("\nSummary");
    println!(
        "{:<10} {:<14} {:<14} {:>7} {:>10} {:>10} {:>10} {:>10} {:>12}",
        "runtime",
        "workload",
        "parameter",
        "samples",
        "mean",
        "p50",
        "p95",
        "stddev",
        "startup_mean"
    );

    for row in summary {
        println!(
            "{:<10} {:<14} {:<14} {:>7} {:>10.3} {:>10.3} {:>10.3} {:>10.3} {:>12.3}",
            row.runtime,
            row.workload,
            row.parameter,
            row.samples,
            row.mean,
            row.p50,
            row.p95,
            row.stddev,
            row.startup_mean,
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Stats {
    mean: f64,
    p50: f64,
    p95: f64,
    stddev: f64,
}

impl Stats {
    fn from_values(values: &[f64]) -> Self {
        let mut sorted = values.to_vec();
        sorted.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
        Self {
            mean: mean(&sorted),
            p50: percentile_nearest_rank(&sorted, 50),
            p95: percentile_nearest_rank(&sorted, 95),
            stddev: stddev(&sorted),
        }
    }
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

fn stddev(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let average = mean(values);
    let variance = values
        .iter()
        .map(|value| {
            let delta = value - average;
            delta * delta
        })
        .sum::<f64>()
        / values.len() as f64;
    variance.sqrt()
}

fn percentile_nearest_rank(sorted_values: &[f64], percentile: usize) -> f64 {
    if sorted_values.is_empty() {
        return 0.0;
    }
    let rank = ((percentile * sorted_values.len()) + 99) / 100;
    let index = rank.saturating_sub(1).min(sorted_values.len() - 1);
    sorted_values[index]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_uses_nearest_rank() {
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(percentile_nearest_rank(&values, 50), 3.0);
        assert_eq!(percentile_nearest_rank(&values, 95), 5.0);
    }

    #[test]
    fn stats_compute_basic_moments() {
        let stats = Stats::from_values(&[10.0, 20.0, 30.0]);
        assert_eq!(stats.mean, 20.0);
        assert_eq!(stats.p50, 20.0);
        assert_eq!(stats.p95, 30.0);
        assert!((stats.stddev - 8.164_965_8).abs() < 0.000_001);
    }

    #[test]
    fn parse_bench_args_uses_separate_plain_and_aot_artifacts() {
        let config = parse_bench_args(Vec::<String>::new()).unwrap();
        assert_eq!(
            config.wasm_artifact,
            PathBuf::from("target/wasm32-wasip1/release/my-app.wasm")
        );
        assert_eq!(config.wasm_aot_artifact, PathBuf::from("my-app-aot.wasm"));
        assert_eq!(
            config.summary_output_path,
            PathBuf::from("target/bench-results/summary.csv")
        );

        let overridden = parse_bench_args([
            "--wasm-artifact",
            "plain.wasm",
            "--wasm-aot-artifact",
            "compiled-aot.so",
            "--summary-output",
            "summary.csv",
        ])
        .unwrap();
        assert_eq!(overridden.wasm_artifact, PathBuf::from("plain.wasm"));
        assert_eq!(
            overridden.wasm_aot_artifact,
            PathBuf::from("compiled-aot.so")
        );
        assert_eq!(overridden.summary_output_path, PathBuf::from("summary.csv"));
    }

    #[test]
    fn build_summary_aggregates_rows() {
        let summary = build_summary(&[
            SampleRecord {
                runtime: "wasmedge-aot",
                workload: "fib".to_string(),
                parameter: "n=40".to_string(),
                sample: 1,
                e2e_ms: 100.0,
                internal_compute_ms: 90.0,
                startup_overhead_ms: 10.0,
                exit_code: 0,
            },
            SampleRecord {
                runtime: "wasmedge-aot",
                workload: "fib".to_string(),
                parameter: "n=40".to_string(),
                sample: 2,
                e2e_ms: 120.0,
                internal_compute_ms: 100.0,
                startup_overhead_ms: 20.0,
                exit_code: 0,
            },
        ]);

        let row = summary
            .iter()
            .find(|row| row.runtime == "wasmedge-aot" && row.workload == "fib")
            .unwrap();
        assert_eq!(row.samples, 2);
        assert_eq!(row.mean, 110.0);
        assert_eq!(row.p50, 100.0);
        assert_eq!(row.p95, 120.0);
        assert_eq!(row.startup_mean, 15.0);
    }
}
