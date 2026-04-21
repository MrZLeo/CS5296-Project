use std::fmt;
use std::thread::sleep;
use std::time::Duration;
use std::time::Instant;

pub const DEFAULT_FIB_N: u32 = 45;
pub const DEFAULT_BENCH_FIB_N: u32 = 40;
pub const DEFAULT_ALLOC_BYTES: usize = 64 * 1024 * 1024;

const APP_USAGE: &str = "\
Usage:
  my-app [--workload noop]
  my-app [--workload fib] [--n <u32>]
  my-app --workload alloc_touch [--bytes <usize>] [--hold-ms <u64>]

Defaults:
  workload=fib
  n=45
  bytes=67108864
  hold_ms=0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Workload {
    Noop,
    Fib { n: u32 },
    AllocTouch { bytes: usize, hold_ms: u64 },
}

impl Workload {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Noop => "noop",
            Self::Fib { .. } => "fib",
            Self::AllocTouch { .. } => "alloc_touch",
        }
    }

    pub fn parameter(&self) -> String {
        match self {
            Self::Noop => "none".to_string(),
            Self::Fib { n } => format!("n={n}"),
            Self::AllocTouch { bytes, hold_ms } => {
                if *hold_ms == 0 {
                    format!("bytes={bytes}")
                } else {
                    format!("bytes={bytes},hold_ms={hold_ms}")
                }
            }
        }
    }

    pub fn cli_args(&self) -> Vec<String> {
        match self {
            Self::Noop => vec!["--workload".to_string(), "noop".to_string()],
            Self::Fib { n } => vec![
                "--workload".to_string(),
                "fib".to_string(),
                "--n".to_string(),
                n.to_string(),
            ],
            Self::AllocTouch { bytes, hold_ms } => {
                let mut args = vec![
                    "--workload".to_string(),
                    "alloc_touch".to_string(),
                    "--bytes".to_string(),
                    bytes.to_string(),
                ];
                if *hold_ms > 0 {
                    args.push("--hold-ms".to_string());
                    args.push(hold_ms.to_string());
                }
                args
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppOutput {
    pub workload: String,
    pub parameter: String,
    pub result_digest: u64,
    pub internal_compute_ms: f64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseAppArgsError {
    HelpRequested,
    Message(String),
}

impl fmt::Display for ParseAppArgsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HelpRequested => f.write_str(APP_USAGE),
            Self::Message(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ParseAppArgsError {}

pub fn app_usage() -> &'static str {
    APP_USAGE
}

pub fn parse_app_args<I, S>(args: I) -> Result<Workload, ParseAppArgsError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<String>>();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return Err(ParseAppArgsError::HelpRequested);
    }

    let mut workload_name: Option<String> = None;
    let mut fib_n: Option<u32> = None;
    let mut alloc_bytes: Option<usize> = None;
    let mut hold_ms: Option<u64> = None;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--workload" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    ParseAppArgsError::Message("missing value for --workload".to_string())
                })?;
                workload_name = Some(value.clone());
                index += 2;
            }
            "--n" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    ParseAppArgsError::Message("missing value for --n".to_string())
                })?;
                fib_n = Some(parse_u32(value, "--n")?);
                index += 2;
            }
            "--bytes" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    ParseAppArgsError::Message("missing value for --bytes".to_string())
                })?;
                alloc_bytes = Some(parse_usize(value, "--bytes")?);
                index += 2;
            }
            "--hold-ms" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    ParseAppArgsError::Message("missing value for --hold-ms".to_string())
                })?;
                hold_ms = Some(parse_u64(value, "--hold-ms")?);
                index += 2;
            }
            unknown => {
                return Err(ParseAppArgsError::Message(format!(
                    "unknown argument: {unknown}\n\n{}",
                    app_usage()
                )));
            }
        }
    }

    let workload = match workload_name.as_deref().unwrap_or("fib") {
        "noop" => {
            if fib_n.is_some() {
                return Err(ParseAppArgsError::Message(
                    "--n is only valid with --workload fib".to_string(),
                ));
            }
            if alloc_bytes.is_some() {
                return Err(ParseAppArgsError::Message(
                    "--bytes is only valid with --workload alloc_touch".to_string(),
                ));
            }
            if hold_ms.is_some() {
                return Err(ParseAppArgsError::Message(
                    "--hold-ms is only valid with --workload alloc_touch".to_string(),
                ));
            }
            Workload::Noop
        }
        "fib" => {
            if alloc_bytes.is_some() {
                return Err(ParseAppArgsError::Message(
                    "--bytes is only valid with --workload alloc_touch".to_string(),
                ));
            }
            if hold_ms.is_some() {
                return Err(ParseAppArgsError::Message(
                    "--hold-ms is only valid with --workload alloc_touch".to_string(),
                ));
            }
            Workload::Fib {
                n: fib_n.unwrap_or(DEFAULT_FIB_N),
            }
        }
        "alloc_touch" => {
            if fib_n.is_some() {
                return Err(ParseAppArgsError::Message(
                    "--n is only valid with --workload fib".to_string(),
                ));
            }
            Workload::AllocTouch {
                bytes: alloc_bytes.unwrap_or(DEFAULT_ALLOC_BYTES),
                hold_ms: hold_ms.unwrap_or(0),
            }
        }
        other => {
            return Err(ParseAppArgsError::Message(format!(
                "unsupported workload: {other}\nexpected one of: noop, fib, alloc_touch"
            )));
        }
    };

    Ok(workload)
}

pub fn run_workload(workload: &Workload) -> AppOutput {
    let compute_start = Instant::now();
    let result_digest = match workload {
        Workload::Noop => 0,
        Workload::Fib { n } => fibonacci(*n) as u64,
        Workload::AllocTouch { bytes, hold_ms } => alloc_touch(*bytes, *hold_ms),
    };

    AppOutput {
        workload: workload.name().to_string(),
        parameter: workload.parameter(),
        result_digest,
        internal_compute_ms: compute_start.elapsed().as_secs_f64() * 1000.0,
    }
}

pub fn render_app_output_json(output: &AppOutput) -> String {
    format!(
        "{{\"workload\":\"{}\",\"parameter\":\"{}\",\"result_digest\":{},\"internal_compute_ms\":{:.6}}}",
        escape_json_string(&output.workload),
        escape_json_string(&output.parameter),
        output.result_digest,
        output.internal_compute_ms,
    )
}

pub fn parse_app_output_json(input: &str) -> Result<AppOutput, String> {
    let trimmed = input.trim();
    if !(trimmed.starts_with('{') && trimmed.ends_with('}')) {
        return Err("application output was not valid JSON object syntax".to_string());
    }

    Ok(AppOutput {
        workload: extract_string_field(trimmed, "workload")?,
        parameter: extract_string_field(trimmed, "parameter")?,
        result_digest: extract_u64_field(trimmed, "result_digest")?,
        internal_compute_ms: extract_f64_field(trimmed, "internal_compute_ms")?,
    })
}

fn parse_u32(value: &str, flag: &str) -> Result<u32, ParseAppArgsError> {
    value.parse::<u32>().map_err(|_| {
        ParseAppArgsError::Message(format!(
            "invalid value for {flag}: {value} (expected unsigned 32-bit integer)"
        ))
    })
}

fn parse_usize(value: &str, flag: &str) -> Result<usize, ParseAppArgsError> {
    value.parse::<usize>().map_err(|_| {
        ParseAppArgsError::Message(format!(
            "invalid value for {flag}: {value} (expected non-negative integer)"
        ))
    })
}

fn parse_u64(value: &str, flag: &str) -> Result<u64, ParseAppArgsError> {
    value.parse::<u64>().map_err(|_| {
        ParseAppArgsError::Message(format!(
            "invalid value for {flag}: {value} (expected unsigned 64-bit integer)"
        ))
    })
}

fn fibonacci(n: u32) -> u32 {
    match n {
        0 => 0,
        1 => 1,
        _ => fibonacci(n - 1) + fibonacci(n - 2),
    }
}

fn alloc_touch(bytes: usize, hold_ms: u64) -> u64 {
    let mut buffer = vec![0_u8; bytes];
    let mut digest = 14_695_981_039_346_656_037_u64 ^ bytes as u64;
    let page_size = 4096usize;

    for index in (0..bytes).step_by(page_size) {
        let value = ((index / page_size) as u8)
            .wrapping_mul(31)
            .wrapping_add(17);
        buffer[index] = value;
        digest = digest.wrapping_mul(1_099_511_628_211);
        digest ^= value as u64;
    }

    if bytes > 0 {
        let tail_index = bytes - 1;
        buffer[tail_index] = buffer[tail_index].wrapping_add(0xA5);
        digest = digest.wrapping_mul(1_099_511_628_211);
        digest ^= buffer[tail_index] as u64;
    }

    if hold_ms > 0 {
        sleep(Duration::from_millis(hold_ms));
    }

    digest ^ buffer.len() as u64
}

fn escape_json_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn extract_string_field(input: &str, key: &str) -> Result<String, String> {
    let raw_value = extract_raw_value(input, key)?;
    if !(raw_value.starts_with('"') && raw_value.ends_with('"')) {
        return Err(format!("field {key} was not a JSON string"));
    }

    Ok(raw_value[1..raw_value.len() - 1]
        .replace("\\\"", "\"")
        .replace("\\\\", "\\"))
}

fn extract_u64_field(input: &str, key: &str) -> Result<u64, String> {
    extract_raw_value(input, key)?
        .parse::<u64>()
        .map_err(|_| format!("field {key} was not a valid u64"))
}

fn extract_f64_field(input: &str, key: &str) -> Result<f64, String> {
    extract_raw_value(input, key)?
        .parse::<f64>()
        .map_err(|_| format!("field {key} was not a valid f64"))
}

fn extract_raw_value<'a>(input: &'a str, key: &str) -> Result<&'a str, String> {
    let pattern = format!("\"{key}\":");
    let value_start = input
        .find(&pattern)
        .map(|index| index + pattern.len())
        .ok_or_else(|| format!("missing field {key}"))?;
    let remaining = &input[value_start..];

    if remaining.starts_with('"') {
        let string_end = find_string_end(remaining)?;
        return Ok(&remaining[..=string_end]);
    }

    let mut end = remaining.len();
    for delimiter in [',', '}'] {
        if let Some(index) = remaining.find(delimiter) {
            end = end.min(index);
        }
    }
    Ok(remaining[..end].trim())
}

fn find_string_end(input: &str) -> Result<usize, String> {
    let mut escaped = false;
    for (index, ch) in input.char_indices().skip(1) {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => return Ok(index),
            _ => {}
        }
    }
    Err("unterminated JSON string".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_args_use_fib_45() {
        assert_eq!(
            parse_app_args(Vec::<String>::new()).unwrap(),
            Workload::Fib { n: 45 }
        );
    }

    #[test]
    fn alloc_touch_args_parse() {
        let args = ["--workload", "alloc_touch", "--bytes", "8192"];
        assert_eq!(
            parse_app_args(args).unwrap(),
            Workload::AllocTouch {
                bytes: 8192,
                hold_ms: 0,
            }
        );
    }

    #[test]
    fn alloc_touch_hold_args_parse() {
        let args = [
            "--workload",
            "alloc_touch",
            "--bytes",
            "8192",
            "--hold-ms",
            "250",
        ];
        assert_eq!(
            parse_app_args(args).unwrap(),
            Workload::AllocTouch {
                bytes: 8192,
                hold_ms: 250,
            }
        );
    }

    #[test]
    fn json_round_trip_preserves_output() {
        let output = AppOutput {
            workload: "fib".to_string(),
            parameter: "n=40".to_string(),
            result_digest: 102_334_155,
            internal_compute_ms: 123.456789,
        };

        let decoded = parse_app_output_json(&render_app_output_json(&output)).unwrap();
        assert_eq!(decoded.workload, output.workload);
        assert_eq!(decoded.parameter, output.parameter);
        assert_eq!(decoded.result_digest, output.result_digest);
        assert!((decoded.internal_compute_ms - output.internal_compute_ms).abs() < 0.000_001);
    }
}
