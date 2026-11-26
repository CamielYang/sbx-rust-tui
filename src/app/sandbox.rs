use serde_json::Value;
use std::process::Command as ProcessCommand;

/// Collected with `sbx exec <name> -- sh -c <script>`. Plain text keeps the
/// command portable across the minimal shells used inside the microVM. CPU
/// usage needs two `/proc/stat` samples, so the guest sleeps for one second
/// between them and reports a percentage of all cores.
const VM_METRICS_SCRIPT: &str = r#"
printf 'cpu=%s\n' "$(nproc 2>/dev/null || printf 0)"
awk '
  function sample(   line, fields, count, i, total, idle) {
    while ((getline line < "/proc/stat") > 0) {
      if (line ~ /^cpu[ \t]/) {
        count = split(line, fields, " ")
        total = 0
        for (i = 2; i <= count; i++) total += fields[i] + 0
        idle = fields[5] + 0 + fields[6] + 0
        close("/proc/stat")
        return total " " idle
      }
    }
    close("/proc/stat")
    return ""
  }
  BEGIN {
    before = sample()
    system("sleep 1")
    after = sample()
    if (before != "" && after != "") {
      split(before, first, " ")
      split(after, second, " ")
      total = second[1] - first[1]
      idle = second[2] - first[2]
      if (total > 0) {
        usage = 100 * (total - idle) / total
        if (usage < 0) usage = 0
        if (usage > 100) usage = 100
        printf "cpu_usage=%d\n", int(usage + 0.5)
      }
    }
  }
  /^MemTotal:/     { total = $2 }
  /^MemFree:/      { free = $2 }
  /^MemAvailable:/ { avail = $2 }
  END {
    if (total != "") print "mem_total_kb=" total
    if (avail != "") print "mem_free_kb=" avail
    else if (free != "") print "mem_free_kb=" free
  }
' /proc/meminfo
df -k
"#;

/// A sandbox as shown in a dashboard block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sandbox {
    pub name: String,
    pub status: String,
}

impl Sandbox {
    pub fn new(name: String, status: String) -> Self {
        Self { name, status }
    }

    pub fn is_running(&self) -> bool {
        self.status.trim().to_ascii_lowercase().contains("running")
    }
}
/// How a host path is attached to the sandbox. The source clone always lands
/// at the well-known source location; every other host path is a direct mount.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountMode {
    Clone,
    Direct,
}

/// A host path exposed inside the sandbox, as reported by `sbx inspect`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostMount {
    pub host_path: String,
    pub mode: MountMode,
}

/// Resource details shown in the info modal. Values come from live commands
/// run inside the microVM; the structured `sbx inspect` output is only a
/// fallback for fields the guest cannot report.
#[derive(Clone, Debug, Default)]
pub struct SandboxDetails {
    pub name: String,
    pub status: String,
    pub cpus: Option<String>,
    /// Percentage of all guest cores currently busy, sampled over one second.
    pub cpu_usage: Option<u8>,
    pub mem_used: Option<String>,
    pub mem_total: Option<String>,
    pub root_used: Option<String>,
    pub root_total: Option<String>,
    pub docker_used: Option<String>,
    pub docker_total: Option<String>,
    /// Host paths visible in the sandbox, with their mount mode.
    pub mounts: Vec<HostMount>,
    pub error: Option<String>,
}

/// Raw KiB values read from the guest. Used space is derived as
/// total - available so every resource fits on one `used / total` row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct VmMetrics {
    pub(super) cpus: Option<String>,
    pub(super) cpu_usage: Option<u8>,
    pub(super) mem_total_kib: Option<u64>,
    pub(super) mem_free_kib: Option<u64>,
    pub(super) root_total_kib: Option<u64>,
    pub(super) root_free_kib: Option<u64>,
    pub(super) docker_total_kib: Option<u64>,
    pub(super) docker_free_kib: Option<u64>,
}

impl VmMetrics {
    /// Format a resource as (used, total). Used is only known when the guest
    /// reported both numbers; a configured total alone is still useful.
    pub(super) fn usage(total: Option<u64>, free: Option<u64>) -> (Option<String>, Option<String>) {
        let used = total
            .zip(free)
            .map(|(total, free)| format_size(total.saturating_sub(free)));
        (used, total.map(format_size))
    }
}

/// Convert the KiB values reported by `/proc/meminfo` and `df -k` into a
/// compact human-readable size.
pub(super) fn format_size(kib: u64) -> String {
    const MIB: u64 = 1024;
    const GIB: u64 = 1024 * MIB;
    const TIB: u64 = 1024 * GIB;

    if kib >= TIB {
        format!("{:.1} TiB", kib as f64 / TIB as f64)
    } else if kib >= GIB {
        format!("{:.1} GiB", kib as f64 / GIB as f64)
    } else if kib >= MIB {
        format!("{:.1} MiB", kib as f64 / MIB as f64)
    } else {
        format!("{kib} KiB")
    }
}

/// Parse the key/value report plus `df -k` table emitted by
/// [`VM_METRICS_SCRIPT`]. The docker mount is preferred at its standard
/// location, with any other mount containing "docker" as a fallback.
pub(super) fn parse_vm_metrics(output: &str) -> VmMetrics {
    let mut metrics = VmMetrics::default();
    // (total_kib, free_kib, is_standard_location)
    let mut docker_mount: Option<(u64, u64, bool)> = None;

    for line in output.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("cpu=") {
            if value.parse::<u64>().is_ok() {
                metrics.cpus = Some(value.to_owned());
            }
            continue;
        }
        if let Some(value) = line.strip_prefix("cpu_usage=") {
            metrics.cpu_usage = value.parse::<u8>().ok().filter(|usage| *usage <= 100);
            continue;
        }
        if let Some(value) = line.strip_prefix("mem_total_kb=") {
            metrics.mem_total_kib = value.parse::<u64>().ok();
            continue;
        }
        if let Some(value) = line.strip_prefix("mem_free_kb=") {
            metrics.mem_free_kib = value.parse::<u64>().ok();
            continue;
        }

        // df rows: Filesystem 1K-blocks Used Available Use% Mounted on
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 6 || fields[0] == "Filesystem" {
            continue;
        }
        let (Ok(total), Ok(free)) = (fields[1].parse::<u64>(), fields[3].parse::<u64>()) else {
            continue;
        };
        let mount = fields[5..].join(" ");
        if mount == "/" {
            metrics.root_total_kib = Some(total);
            metrics.root_free_kib = Some(free);
        } else if mount == "/var/lib/docker" {
            docker_mount = Some((total, free, true));
        } else if mount.contains("docker")
            && docker_mount.as_ref().is_none_or(|(_, _, exact)| !*exact)
        {
            docker_mount = Some((total, free, false));
        }
    }

    if let Some((total, free, _)) = docker_mount {
        metrics.docker_total_kib = Some(total);
        metrics.docker_free_kib = Some(free);
    }
    metrics
}

/// Fetch sandboxes with their status from `sbx ls --format json`. Falls back
/// to `sbx ls -q` when structured output is unavailable.
pub fn fetch_sandboxes() -> Result<Vec<Sandbox>, String> {
    let structured = ProcessCommand::new("sbx")
        .args(["ls", "--format", "json"])
        .output()
        .map_err(|error| error.to_string())?;

    if structured.status.success()
        && let Ok(value) = serde_json::from_slice::<Value>(&structured.stdout)
    {
        let rows = value
            .as_array()
            .or_else(|| value.get("sandboxes").and_then(Value::as_array));
        if let Some(rows) = rows {
            return Ok(rows
                .iter()
                .filter_map(|row| {
                    let name = json_text(row, &["name", "sandbox_name"])?;
                    let status = json_text(row, &["status", "state"])
                        .unwrap_or_else(|| "unknown".to_owned());
                    Some(Sandbox::new(name, status))
                })
                .collect());
        }
    }

    let output = ProcessCommand::new("sbx")
        .args(["ls", "-q"])
        .output()
        .map_err(|error| error.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if stderr.is_empty() {
            format!("sbx ls exited with {}.", output.status)
        } else {
            stderr
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|name| Sandbox::new(name.to_owned(), "unknown".to_owned()))
        .collect())
}

/// Read details for one sandbox. `sbx inspect` supplies the configured
/// values; a command inside the microVM supplies the live numbers. Info calls
/// this on a worker when the dialog opens and every ten seconds after that.
pub fn inspect_sandbox(sandbox: &Sandbox) -> SandboxDetails {
    let mut details = SandboxDetails {
        name: sandbox.name.clone(),
        status: sandbox.status.clone(),
        ..SandboxDetails::default()
    };

    load_inspect_details(sandbox, &mut details);

    let output = ProcessCommand::new("sbx")
        .args(["exec", &sandbox.name, "--", "sh", "-c", VM_METRICS_SCRIPT])
        .output();

    match output {
        Ok(output) if output.status.success() => {
            let metrics = parse_vm_metrics(&String::from_utf8_lossy(&output.stdout));
            override_if_present(&mut details.cpus, metrics.cpus);
            details.cpu_usage = metrics.cpu_usage;

            let (mem_used, mem_total) =
                VmMetrics::usage(metrics.mem_total_kib, metrics.mem_free_kib);
            override_if_present(&mut details.mem_used, mem_used);
            override_if_present(&mut details.mem_total, mem_total);

            let (root_used, root_total) =
                VmMetrics::usage(metrics.root_total_kib, metrics.root_free_kib);
            override_if_present(&mut details.root_used, root_used);
            override_if_present(&mut details.root_total, root_total);

            let (docker_used, docker_total) =
                VmMetrics::usage(metrics.docker_total_kib, metrics.docker_free_kib);
            override_if_present(&mut details.docker_used, docker_used);
            override_if_present(&mut details.docker_total, docker_total);
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            push_error(
                &mut details,
                if stderr.is_empty() {
                    format!(
                        "Could not read live metrics (sbx exec exited with {}).",
                        output.status
                    )
                } else {
                    format!("Could not read live metrics: {stderr}")
                },
            );
        }
        Err(error) => {
            push_error(
                &mut details,
                format!("Could not read live metrics: {error}"),
            );
        }
    }

    details
}

/// Fill in configured values from `sbx inspect --format json`. These are used
/// as fallbacks when the guest cannot report a field.
fn load_inspect_details(sandbox: &Sandbox, details: &mut SandboxDetails) {
    let output = match ProcessCommand::new("sbx")
        .args(["inspect", &sandbox.name, "--format", "json"])
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            push_error(details, format!("Could not inspect sandbox: {error}"));
            return;
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        push_error(
            details,
            if stderr.is_empty() {
                format!("sbx inspect exited with {}.", output.status)
            } else {
                stderr
            },
        );
        return;
    }

    let value: Value = match serde_json::from_slice(&output.stdout) {
        Ok(value) => value,
        Err(error) => {
            push_error(
                details,
                format!("Could not parse sbx inspect output: {error}"),
            );
            return;
        }
    };

    details.cpus = json_text(&value, &["cpus", "cpu", "cpu_count", "vcpus"]);
    details.mem_total = json_text(
        &value,
        &["memory", "memoryMiB", "memory_mib", "memory_mb", "mem"],
    );
    details.root_total = json_text(
        &value,
        &[
            "root_size",
            "rootSize",
            "rootfs",
            "root_filesystem_size",
            "rootFilesystemSize",
        ],
    );
    details.docker_total = json_text(
        &value,
        &[
            "docker_size",
            "dockerSize",
            "docker",
            "dind_volume_size",
            "dindVolumeSize",
        ],
    );
    details.mounts = parse_host_mounts(&value);
}

/// Read `host_mounts` from `sbx inspect`. A mount at `/run/sandbox/source` is
/// the repository clone; every other host mount is a direct mount.
pub(super) fn parse_host_mounts(value: &Value) -> Vec<HostMount> {
    let Some(rows) = find_key(value, "host_mounts", 0).and_then(Value::as_array) else {
        return Vec::new();
    };

    rows.iter()
        .filter_map(|row| {
            let host_path = json_text(row, &["host_path", "hostPath", "host", "source", "path"])?;
            let target = json_text(row, &["container_target", "containerTarget", "target"]);
            let mode = match target.as_deref() {
                Some("/run/sandbox/source") => MountMode::Clone,
                _ => MountMode::Direct,
            };
            Some(HostMount { host_path, mode })
        })
        .collect()
}

fn override_if_present(target: &mut Option<String>, value: Option<String>) {
    if value.is_some() {
        *target = value;
    }
}

pub(super) fn push_error(details: &mut SandboxDetails, error: String) {
    match &mut details.error {
        Some(existing) => {
            existing.push_str("; ");
            existing.push_str(&error);
        }
        None => details.error = Some(error),
    }
}

/// Find a key near the root of a JSON document. `sbx inspect` nests the same
/// fields at different depths depending on the sandbox, so a bounded search
/// is more robust than a fixed path.
fn find_key<'a>(value: &'a Value, key: &str, depth: usize) -> Option<&'a Value> {
    if depth > 5 {
        return None;
    }
    if let Some(map) = value.as_object() {
        if let Some(found) = map.get(key) {
            return Some(found);
        }
        for child in map.values() {
            if let Some(found) = find_key(child, key, depth + 1) {
                return Some(found);
            }
        }
    } else if let Some(items) = value.as_array() {
        for child in items {
            if let Some(found) = find_key(child, key, depth + 1) {
                return Some(found);
            }
        }
    }
    None
}

pub(super) fn json_text(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| match find_key(value, key, 0)? {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(boolean) => Some(boolean.to_string()),
        _ => None,
    })
}
