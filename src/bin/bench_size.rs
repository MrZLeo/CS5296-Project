use std::env;
use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

use my_app::{Workload, DEFAULT_ALLOC_BYTES};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const DEFAULT_DOCKER_IMAGE: &str = "my-docker-app:latest";
const DEFAULT_DOCKER_APP_PATH: &str = "/usr/local/cargo/bin/my-app";
const DEFAULT_WASM_ARTIFACT: &str = "target/wasm32-wasip1/release/my-app.wasm";
const DEFAULT_WASM_AOT_ARTIFACT: &str = "my-app-aot.wasm";
const DEFAULT_OUTPUT_PATH: &str = "target/bench-results/space-size.csv";
const DEFAULT_RUNTIME_HOLD_MS: u64 = 1_200;
const DEFAULT_RUNTIME_POLL_MS: u64 = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
struct BenchConfig {
    docker_image: String,
    docker_app_path: String,
    wasm_artifact: PathBuf,
    wasm_aot_artifact: PathBuf,
    output_path: PathBuf,
    runtime_bytes: usize,
    runtime_hold_ms: u64,
    runtime_poll_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Runtime {
    WasmEdgeWasm,
    WasmEdgeAot,
    Docker,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Metric {
    ArtifactOnly,
    FullDeploySize,
    RuntimePeakRss,
}

impl Metric {
    fn name(self) -> &'static str {
        match self {
            Self::ArtifactOnly => "artifact_only",
            Self::FullDeploySize => "full_deploy_size",
            Self::RuntimePeakRss => "runtime_peak_rss",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct SizeRecord {
    runtime: &'static str,
    metric: &'static str,
    kind: &'static str,
    source: String,
    value_bytes: u64,
    value_mib: f64,
}

fn main() {
    let config = match parse_size_args(std::env::args().skip(1)) {
        Ok(config) => config,
        Err(error) if error == "--help" => {
            println!("{}", size_usage());
            return;
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };

    if let Err(error) = run_size_benchmark(&config) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run_size_benchmark(config: &BenchConfig) -> Result<(), String> {
    for runtime in Runtime::all() {
        runtime.preflight(config)?;
    }

    let wasmedge_runtime = WasmEdgeRuntime::discover()?;

    let mut records = Vec::with_capacity(Runtime::all().len() * 3);
    for runtime in Runtime::all() {
        let artifact_record = runtime.measure_artifact_only(config)?;
        print_record(&artifact_record);
        records.push(artifact_record);

        let full_deploy_record = runtime.measure_full_deploy_size(config, &wasmedge_runtime)?;
        print_record(&full_deploy_record);
        records.push(full_deploy_record);

        let runtime_record = runtime.measure_runtime_peak_rss(config)?;
        print_record(&runtime_record);
        records.push(runtime_record);
    }

    write_csv(config.output_path.as_path(), &records)?;
    print_summary(&records);
    println!("\nSize CSV written to {}", config.output_path.display());
    Ok(())
}

fn parse_size_args<I, S>(args: I) -> Result<BenchConfig, String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<String>>();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return Err("--help".to_string());
    }

    let mut docker_image = DEFAULT_DOCKER_IMAGE.to_string();
    let mut docker_app_path = DEFAULT_DOCKER_APP_PATH.to_string();
    let mut wasm_artifact = PathBuf::from(DEFAULT_WASM_ARTIFACT);
    let mut wasm_aot_artifact = PathBuf::from(DEFAULT_WASM_AOT_ARTIFACT);
    let mut output_path = PathBuf::from(DEFAULT_OUTPUT_PATH);
    let mut runtime_bytes = DEFAULT_ALLOC_BYTES;
    let mut runtime_hold_ms = DEFAULT_RUNTIME_HOLD_MS;
    let mut runtime_poll_ms = DEFAULT_RUNTIME_POLL_MS;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--docker-image" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "missing value for --docker-image".to_string())?;
                docker_image = value.clone();
                index += 2;
            }
            "--docker-app-path" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "missing value for --docker-app-path".to_string())?;
                docker_app_path = value.clone();
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
            "--runtime-bytes" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "missing value for --runtime-bytes".to_string())?;
                runtime_bytes = parse_usize_arg(value, "--runtime-bytes")?;
                index += 2;
            }
            "--hold-ms" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "missing value for --hold-ms".to_string())?;
                runtime_hold_ms = parse_u64_arg(value, "--hold-ms")?;
                index += 2;
            }
            "--poll-ms" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "missing value for --poll-ms".to_string())?;
                runtime_poll_ms = parse_u64_arg(value, "--poll-ms")?;
                if runtime_poll_ms == 0 {
                    return Err("--poll-ms must be at least 1".to_string());
                }
                index += 2;
            }
            unknown => {
                return Err(format!("unknown argument: {unknown}\n\n{}", size_usage()));
            }
        }
    }

    Ok(BenchConfig {
        docker_image,
        docker_app_path,
        wasm_artifact,
        wasm_aot_artifact,
        output_path,
        runtime_bytes,
        runtime_hold_ms,
        runtime_poll_ms,
    })
}

fn parse_usize_arg(value: &str, flag: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|_| format!("invalid value for {flag}: {value}"))
}

fn parse_u64_arg(value: &str, flag: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("invalid value for {flag}: {value}"))
}

fn size_usage() -> &'static str {
    "\
Usage:
  cargo run --bin bench_size --release -- [options]

Options:
  --docker-image <tag>      Docker image tag to inspect (default: my-docker-app:latest)
  --docker-app-path <path>  Application binary path inside the Docker image (default: /usr/local/cargo/bin/my-app)
  --wasm-artifact <path>    Plain Wasm artifact path (default: target/wasm32-wasip1/release/my-app.wasm)
  --wasm-aot-artifact <path> AOT WasmEdge artifact path (default: my-app-aot.wasm)
  --output <path>           Output CSV path (default: target/bench-results/space-size.csv)
  --runtime-bytes <usize>   Allocation size used by the runtime RSS probe (default: 67108864)
  --hold-ms <u64>           Time to hold the allocation before exit (default: 1200)
  --poll-ms <u64>           Poll interval for RSS sampling (default: 100)"
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
                ensure_file_exists(&config.wasm_artifact, "plain Wasm artifact")?;
                check_command_success("wasmedge", &["--version"], None)
                    .map_err(|error| format!("WasmEdge preflight failed: {error}"))
            }
            Self::WasmEdgeAot => {
                ensure_file_exists(&config.wasm_aot_artifact, "AOT Wasm artifact")?;
                check_command_success("wasmedge", &["--version"], None)
                    .map_err(|error| format!("WasmEdge preflight failed: {error}"))
            }
            Self::Docker => check_command_success(
                "docker",
                &["image", "inspect", config.docker_image.as_str()],
                Some(format!(
                    "Build it with: docker build -t {} .",
                    config.docker_image
                )),
            )
            .map_err(|error| format!("Docker preflight failed: {error}")),
        }
    }

    fn measure_artifact_only(self, config: &BenchConfig) -> Result<SizeRecord, String> {
        match self {
            Self::WasmEdgeWasm => build_record(
                self.name(),
                Metric::ArtifactOnly,
                "artifact",
                config.wasm_artifact.display().to_string(),
                file_size_bytes(&config.wasm_artifact)?,
            ),
            Self::WasmEdgeAot => build_record(
                self.name(),
                Metric::ArtifactOnly,
                "artifact",
                config.wasm_aot_artifact.display().to_string(),
                file_size_bytes(&config.wasm_aot_artifact)?,
            ),
            Self::Docker => build_record(
                self.name(),
                Metric::ArtifactOnly,
                "binary",
                config.docker_app_path.clone(),
                docker_binary_size_bytes(
                    config.docker_image.as_str(),
                    config.docker_app_path.as_str(),
                )?,
            ),
        }
    }

    fn measure_full_deploy_size(
        self,
        config: &BenchConfig,
        wasmedge_runtime: &WasmEdgeRuntime,
    ) -> Result<SizeRecord, String> {
        match self {
            Self::WasmEdgeWasm => {
                let artifact_bytes = file_size_bytes(&config.wasm_artifact)?;
                let value_bytes = artifact_bytes + wasmedge_runtime.size_bytes;
                build_record(
                    self.name(),
                    Metric::FullDeploySize,
                    "runtime+artifact",
                    format!(
                        "{} + {}",
                        config.wasm_artifact.display(),
                        wasmedge_runtime.root.display()
                    ),
                    value_bytes,
                )
            }
            Self::WasmEdgeAot => {
                let artifact_bytes = file_size_bytes(&config.wasm_aot_artifact)?;
                let value_bytes = artifact_bytes + wasmedge_runtime.size_bytes;
                build_record(
                    self.name(),
                    Metric::FullDeploySize,
                    "runtime+artifact",
                    format!(
                        "{} + {}",
                        config.wasm_aot_artifact.display(),
                        wasmedge_runtime.root.display()
                    ),
                    value_bytes,
                )
            }
            Self::Docker => build_record(
                self.name(),
                Metric::FullDeploySize,
                "image",
                config.docker_image.clone(),
                docker_image_size_bytes(config.docker_image.as_str())?,
            ),
        }
    }

    fn measure_runtime_peak_rss(self, config: &BenchConfig) -> Result<SizeRecord, String> {
        let workload = runtime_probe_workload(config);
        let source = workload.parameter();

        match self {
            Self::WasmEdgeWasm => build_record(
                self.name(),
                Metric::RuntimePeakRss,
                "process",
                source,
                measure_host_process_peak_rss(
                    "wasmedge",
                    command_args_from_path(&config.wasm_artifact, &workload),
                    config.runtime_poll_ms,
                )?,
            ),
            Self::WasmEdgeAot => build_record(
                self.name(),
                Metric::RuntimePeakRss,
                "process",
                source,
                measure_host_process_peak_rss(
                    "wasmedge",
                    command_args_from_path(&config.wasm_aot_artifact, &workload),
                    config.runtime_poll_ms,
                )?,
            ),
            Self::Docker => build_record(
                self.name(),
                Metric::RuntimePeakRss,
                "container",
                source,
                measure_docker_container_peak_rss(
                    config.docker_image.as_str(),
                    workload.cli_args(),
                    config.runtime_poll_ms,
                )?,
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WasmEdgeRuntime {
    root: PathBuf,
    size_bytes: u64,
}

impl WasmEdgeRuntime {
    fn discover() -> Result<Self, String> {
        let binary = find_program_in_path("wasmedge")?;
        let root = binary
            .parent()
            .and_then(|parent| parent.parent())
            .ok_or_else(|| {
                format!(
                    "failed to derive WasmEdge runtime root from {}",
                    binary.display()
                )
            })?
            .to_path_buf();
        let size_bytes = directory_size_bytes(&root)?;
        Ok(Self { root, size_bytes })
    }
}

fn find_program_in_path(program: &str) -> Result<PathBuf, String> {
    let path = env::var_os("PATH").ok_or_else(|| "PATH is not set".to_string())?;
    for directory in env::split_paths(&path) {
        let candidate = directory.join(program);
        if is_executable_file(&candidate) {
            return fs::canonicalize(&candidate)
                .map_err(|error| format!("failed to resolve {}: {error}", candidate.display()));
        }
    }
    Err(format!("failed to find `{program}` in PATH"))
}

fn is_executable_file(path: &Path) -> bool {
    match fs::metadata(path) {
        Ok(metadata) => is_executable_metadata(path, &metadata),
        Err(_) => false,
    }
}

#[cfg(unix)]
fn is_executable_metadata(_path: &Path, metadata: &fs::Metadata) -> bool {
    metadata.is_file() && (metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(windows)]
fn is_executable_metadata(path: &Path, metadata: &fs::Metadata) -> bool {
    if !metadata.is_file() {
        return false;
    }

    let extension = match path.extension().and_then(|value| value.to_str()) {
        Some(extension) => extension,
        None => return false,
    };

    env::var_os("PATHEXT")
        .and_then(|value| value.into_string().ok())
        .map(|value| {
            value.split(';').any(|candidate| {
                candidate
                    .strip_prefix('.')
                    .is_some_and(|candidate| candidate.eq_ignore_ascii_case(extension))
            })
        })
        .unwrap_or(false)
}

#[cfg(not(any(unix, windows)))]
fn is_executable_metadata(_path: &Path, metadata: &fs::Metadata) -> bool {
    metadata.is_file()
}

fn file_size_bytes(path: &Path) -> Result<u64, String> {
    fs::metadata(path)
        .map_err(|error| format!("failed to read {} metadata: {error}", path.display()))?
        .len()
        .try_into()
        .map_err(|_| format!("file size overflow for {}", path.display()))
}

fn directory_size_bytes(path: &Path) -> Result<u64, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("failed to read {} metadata: {error}", path.display()))?;
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    if !metadata.is_dir() {
        return Err(format!(
            "path is neither file nor directory: {}",
            path.display()
        ));
    }

    let mut total = 0_u64;
    let entries = fs::read_dir(path)
        .map_err(|error| format!("failed to read directory {}: {error}", path.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
        total = total
            .checked_add(directory_size_bytes(&entry.path())?)
            .ok_or_else(|| format!("directory size overflow for {}", path.display()))?;
    }
    Ok(total)
}

fn runtime_probe_workload(config: &BenchConfig) -> Workload {
    Workload::AllocTouch {
        bytes: config.runtime_bytes,
        hold_ms: config.runtime_hold_ms,
    }
}

fn command_args_from_path(path: &Path, workload: &Workload) -> Vec<String> {
    let mut args = vec![path.display().to_string()];
    args.extend(workload.cli_args());
    args
}

fn ensure_file_exists(path: &Path, label: &str) -> Result<(), String> {
    if path.is_file() {
        Ok(())
    } else {
        Err(format!("missing {label}: {}", path.display()))
    }
}

fn build_record(
    runtime: &'static str,
    metric: Metric,
    kind: &'static str,
    source: String,
    value_bytes: u64,
) -> Result<SizeRecord, String> {
    Ok(SizeRecord {
        runtime,
        metric: metric.name(),
        kind,
        source,
        value_bytes,
        value_mib: bytes_to_mib(value_bytes),
    })
}

fn docker_binary_size_bytes(image: &str, path: &str) -> Result<u64, String> {
    let script = format!("wc -c <'{}'", shell_single_quote(path));
    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            "--entrypoint",
            "/bin/sh",
            image,
            "-lc",
            script.as_str(),
        ])
        .output()
        .map_err(|error| {
            format!("failed to inspect binary {path} in Docker image {image}: {error}")
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "docker run --rm --entrypoint /bin/sh {} failed with exit code {}: {}",
            image,
            output.status.code().unwrap_or(-1),
            stderr.trim()
        ));
    }

    parse_u64_from_stdout(
        &String::from_utf8_lossy(&output.stdout),
        "docker app binary size",
    )
}

fn shell_single_quote(value: &str) -> String {
    value.replace('\'', "'\"'\"'")
}

fn docker_image_size_bytes(image: &str) -> Result<u64, String> {
    let output = Command::new("docker")
        .args(["image", "inspect", image, "--format", "{{.Size}}"])
        .output()
        .map_err(|error| format!("failed to inspect Docker image {image}: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "docker image inspect {} failed with exit code {}: {}",
            image,
            output.status.code().unwrap_or(-1),
            stderr.trim()
        ));
    }

    parse_u64_from_stdout(
        &String::from_utf8_lossy(&output.stdout),
        "docker image size",
    )
}

fn measure_host_process_peak_rss(
    program: &str,
    args: Vec<String>,
    poll_ms: u64,
) -> Result<u64, String> {
    let mut child = Command::new(program)
        .args(&args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to launch `{program}`: {error}"))?;

    let pid = child.id();
    let mut peak_rss_kib = 0_u64;
    loop {
        if let Some(rss_kib) = read_process_rss_kib(pid)? {
            peak_rss_kib = peak_rss_kib.max(rss_kib);
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                let stderr = read_child_pipe(child.stderr.take())?;
                if !status.success() {
                    return Err(format!(
                        "`{} {}` failed with exit code {}: {}",
                        program,
                        args.join(" "),
                        status.code().unwrap_or(-1),
                        stderr.trim()
                    ));
                }
                break;
            }
            Ok(None) => sleep(Duration::from_millis(poll_ms.max(1))),
            Err(error) => return Err(format!("failed to poll `{program}`: {error}")),
        }
    }

    Ok(kib_to_bytes(peak_rss_kib))
}

fn read_process_rss_kib(pid: u32) -> Result<Option<u64>, String> {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .map_err(|error| format!("failed to run `ps` for pid {pid}: {error}"))?;
    if !output.status.success() {
        return Ok(None);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let rss_kib = trimmed
        .parse::<u64>()
        .map_err(|_| format!("unexpected `ps` rss output for pid {pid}: {trimmed}"))?;
    Ok(Some(rss_kib))
}

fn measure_docker_container_peak_rss(
    image: &str,
    args: Vec<String>,
    poll_ms: u64,
) -> Result<u64, String> {
    let output = Command::new("docker")
        .arg("run")
        .arg("-d")
        .arg(image)
        .args(&args)
        .output()
        .map_err(|error| format!("failed to start Docker container for {image}: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "docker run -d {} failed with exit code {}: {}",
            image,
            output.status.code().unwrap_or(-1),
            stderr.trim()
        ));
    }

    let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if container_id.is_empty() {
        return Err(format!(
            "docker run -d {image} returned an empty container id"
        ));
    }

    let peak_result = poll_docker_peak_rss(container_id.as_str(), poll_ms);
    let exit_result = docker_wait(container_id.as_str());
    let cleanup_result = docker_remove_force(container_id.as_str());

    if let Err(error) = cleanup_result {
        eprintln!(
            "warning: failed to remove container {}: {}",
            container_id, error
        );
    }

    let peak_bytes = peak_result?;
    let exit_code = exit_result?;
    if exit_code != 0 {
        let logs = docker_logs(container_id.as_str()).unwrap_or_else(|_| String::new());
        return Err(format!(
            "container {} exited with {}: {}",
            container_id,
            exit_code,
            logs.trim()
        ));
    }
    Ok(peak_bytes)
}

fn poll_docker_peak_rss(container_id: &str, poll_ms: u64) -> Result<u64, String> {
    let mut peak_bytes = 0_u64;

    loop {
        let state = docker_container_state(container_id)?;
        match state.as_str() {
            "created" | "running" | "restarting" => {
                if let Some(current_bytes) = docker_container_memory_usage_bytes(container_id)? {
                    peak_bytes = peak_bytes.max(current_bytes);
                }
                sleep(Duration::from_millis(poll_ms.max(1)));
            }
            "exited" | "dead" => break,
            other => {
                return Err(format!(
                    "unexpected Docker state for {container_id}: {other}"
                ))
            }
        }
    }

    Ok(peak_bytes)
}

fn docker_container_state(container_id: &str) -> Result<String, String> {
    let output = Command::new("docker")
        .args(["inspect", "-f", "{{.State.Status}}", container_id])
        .output()
        .map_err(|error| format!("failed to inspect container {container_id}: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "docker inspect {} failed with exit code {}: {}",
            container_id,
            output.status.code().unwrap_or(-1),
            stderr.trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn docker_container_memory_usage_bytes(container_id: &str) -> Result<Option<u64>, String> {
    let output = Command::new("docker")
        .args([
            "stats",
            "--no-stream",
            "--format",
            "{{.MemUsage}}",
            container_id,
        ])
        .output()
        .map_err(|error| format!("failed to read docker stats for {container_id}: {error}"))?;
    if !output.status.success() {
        return Ok(None);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let usage_part = trimmed.split('/').next().unwrap_or(trimmed).trim();
    Ok(Some(parse_human_size_to_bytes(usage_part)?))
}

fn docker_wait(container_id: &str) -> Result<i32, String> {
    let output = Command::new("docker")
        .args(["wait", container_id])
        .output()
        .map_err(|error| format!("failed to wait for container {container_id}: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "docker wait {} failed with exit code {}: {}",
            container_id,
            output.status.code().unwrap_or(-1),
            stderr.trim()
        ));
    }

    parse_i32_from_stdout(&String::from_utf8_lossy(&output.stdout), "docker wait")
}

fn docker_logs(container_id: &str) -> Result<String, String> {
    let output = Command::new("docker")
        .args(["logs", container_id])
        .output()
        .map_err(|error| format!("failed to read logs for container {container_id}: {error}"))?;
    let mut combined = String::new();
    combined.push_str(&String::from_utf8_lossy(&output.stdout));
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(combined)
}

fn docker_remove_force(container_id: &str) -> Result<(), String> {
    let output = Command::new("docker")
        .args(["rm", "-f", container_id])
        .output()
        .map_err(|error| format!("failed to remove container {container_id}: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "docker rm -f {} failed with exit code {}: {}",
            container_id,
            output.status.code().unwrap_or(-1),
            stderr.trim()
        ))
    }
}

fn parse_u64_from_stdout(stdout: &str, label: &str) -> Result<u64, String> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Err(format!("{label} returned an empty value"));
    }
    trimmed
        .parse::<u64>()
        .map_err(|_| format!("{label} returned invalid value: {trimmed}"))
}

fn parse_i32_from_stdout(stdout: &str, label: &str) -> Result<i32, String> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Err(format!("{label} returned an empty value"));
    }
    trimmed
        .parse::<i32>()
        .map_err(|_| format!("{label} returned invalid value: {trimmed}"))
}

fn parse_human_size_to_bytes(input: &str) -> Result<u64, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("size string was empty".to_string());
    }

    let number_end = trimmed
        .find(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
        .unwrap_or(trimmed.len());
    let number = trimmed[..number_end]
        .parse::<f64>()
        .map_err(|_| format!("invalid size value: {trimmed}"))?;
    let unit = trimmed[number_end..].trim();

    let multiplier = match unit {
        "" | "B" => 1.0,
        "KB" | "KiB" | "kB" => 1024.0,
        "MB" | "MiB" => 1024.0 * 1024.0,
        "GB" | "GiB" => 1024.0 * 1024.0 * 1024.0,
        "TB" | "TiB" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        other => return Err(format!("unsupported size unit: {other}")),
    };

    Ok((number * multiplier).round() as u64)
}

fn read_child_pipe(pipe: Option<impl Read>) -> Result<String, String> {
    let mut pipe = match pipe {
        Some(pipe) => pipe,
        None => return Ok(String::new()),
    };

    let mut content = String::new();
    pipe.read_to_string(&mut content)
        .map_err(|error| format!("failed to read child output: {error}"))?;
    Ok(content)
}

fn kib_to_bytes(kib: u64) -> u64 {
    kib.saturating_mul(1024)
}

fn bytes_to_mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
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
                "failed to create output directory {}: {error}",
                parent.display()
            )
        })?;
    }
    Ok(())
}

fn write_csv(path: &Path, records: &[SizeRecord]) -> Result<(), String> {
    create_parent_dir(path)?;

    let file = File::create(path)
        .map_err(|error| format!("failed to create size CSV {}: {error}", path.display()))?;
    let mut writer = BufWriter::new(file);
    writeln!(writer, "runtime,metric,kind,source,value_bytes,value_mib")
        .map_err(|error| format!("failed to write CSV header: {error}"))?;

    for record in records {
        writeln!(
            writer,
            "{},{},{},{},{},{:.6}",
            csv_escape(record.runtime),
            csv_escape(record.metric),
            csv_escape(record.kind),
            csv_escape(record.source.as_str()),
            record.value_bytes,
            record.value_mib
        )
        .map_err(|error| format!("failed to write CSV row: {error}"))?;
    }

    writer
        .flush()
        .map_err(|error| format!("failed to flush CSV writer: {error}"))?;
    Ok(())
}

fn csv_escape(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn print_record(record: &SizeRecord) {
    println!(
        "{:<14} {:<17} {:<18} {:>12} bytes ({:>8.3} MiB) {}",
        record.runtime,
        record.metric,
        record.kind,
        record.value_bytes,
        record.value_mib,
        record.source
    );
}

fn print_summary(records: &[SizeRecord]) {
    println!("\nSpace Size Summary");
    println!(
        "{:<14} {:<17} {:<18} {:>12} {:>12} source",
        "runtime", "metric", "kind", "value_bytes", "value_mib"
    );
    for record in records {
        println!(
            "{:<14} {:<17} {:<18} {:>12} {:>12.3} {}",
            record.runtime,
            record.metric,
            record.kind,
            record.value_bytes,
            record.value_mib,
            record.source
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_size_args_uses_defaults_and_overrides() {
        let config = parse_size_args(Vec::<String>::new()).unwrap();
        assert_eq!(config.docker_image, "my-docker-app:latest");
        assert_eq!(config.docker_app_path, "/usr/local/cargo/bin/my-app");
        assert_eq!(
            config.wasm_artifact,
            PathBuf::from("target/wasm32-wasip1/release/my-app.wasm")
        );
        assert_eq!(config.wasm_aot_artifact, PathBuf::from("my-app-aot.wasm"));
        assert_eq!(
            config.output_path,
            PathBuf::from("target/bench-results/space-size.csv")
        );
        assert_eq!(config.runtime_bytes, DEFAULT_ALLOC_BYTES);
        assert_eq!(config.runtime_hold_ms, DEFAULT_RUNTIME_HOLD_MS);
        assert_eq!(config.runtime_poll_ms, DEFAULT_RUNTIME_POLL_MS);

        let overridden = parse_size_args([
            "--docker-image",
            "custom-image:latest",
            "--docker-app-path",
            "/app/custom",
            "--wasm-artifact",
            "plain.wasm",
            "--wasm-aot-artifact",
            "compiled-aot.so",
            "--output",
            "space.csv",
            "--runtime-bytes",
            "2048",
            "--hold-ms",
            "250",
            "--poll-ms",
            "5",
        ])
        .unwrap();

        assert_eq!(overridden.docker_image, "custom-image:latest");
        assert_eq!(overridden.docker_app_path, "/app/custom");
        assert_eq!(overridden.wasm_artifact, PathBuf::from("plain.wasm"));
        assert_eq!(
            overridden.wasm_aot_artifact,
            PathBuf::from("compiled-aot.so")
        );
        assert_eq!(overridden.output_path, PathBuf::from("space.csv"));
        assert_eq!(overridden.runtime_bytes, 2048);
        assert_eq!(overridden.runtime_hold_ms, 250);
        assert_eq!(overridden.runtime_poll_ms, 5);
    }

    #[test]
    fn runtime_probe_uses_alloc_touch_hold() {
        let workload = runtime_probe_workload(&BenchConfig {
            docker_image: "image".to_string(),
            docker_app_path: "/usr/local/bin/app".to_string(),
            wasm_artifact: PathBuf::from("plain.wasm"),
            wasm_aot_artifact: PathBuf::from("aot.wasm"),
            output_path: PathBuf::from("out.csv"),
            runtime_bytes: 4096,
            runtime_hold_ms: 300,
            runtime_poll_ms: 10,
        });

        assert_eq!(
            workload,
            Workload::AllocTouch {
                bytes: 4096,
                hold_ms: 300,
            }
        );
    }

    #[test]
    fn parse_human_size_supports_binary_units() {
        assert_eq!(parse_human_size_to_bytes("512B").unwrap(), 512);
        assert_eq!(parse_human_size_to_bytes("1KiB").unwrap(), 1024);
        assert_eq!(parse_human_size_to_bytes("1.5MiB").unwrap(), 1_572_864);
        assert_eq!(parse_human_size_to_bytes("2 GiB").unwrap(), 2_147_483_648);
    }

    #[test]
    fn csv_escape_quotes_fields_with_commas() {
        assert_eq!(csv_escape("plain"), "plain");
        assert_eq!(csv_escape("bytes=1,hold_ms=2"), "\"bytes=1,hold_ms=2\"");
        assert_eq!(csv_escape("with\"quote"), "\"with\"\"quote\"");
    }

    #[test]
    fn shell_single_quote_escapes_quotes() {
        assert_eq!(shell_single_quote("/plain/path"), "/plain/path");
        assert_eq!(shell_single_quote("/a'b"), "/a'\"'\"'b");
    }
}
