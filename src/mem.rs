//! Memory accounting from /proc smaps_rollup and cgroup v2 (reported in /status).
use serde::Serialize;

#[derive(Serialize, Default, Clone, Debug)]
pub struct ProcMem {
    pub pss_kb: u64,
    pub pss_anon_kb: u64,
    pub pss_file_kb: u64,
    pub rss_kb: u64,
}

pub fn proc_mem(pid: &str) -> Option<ProcMem> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).ok()?;
    let g = |k: &str| {
        s.lines()
            .find(|l| l.starts_with(k))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    };
    Some(ProcMem {
        pss_kb: g("Pss:"),
        pss_anon_kb: g("Pss_Anon:"),
        pss_file_kb: g("Pss_File:"),
        rss_kb: g("Rss:"),
    })
}

pub fn cgroup_current_kb() -> Option<u64> {
    std::fs::read_to_string("/sys/fs/cgroup/memory.current")
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(|b| b / 1024)
}

/// Sum of PSS over every process visible in this PID namespace (inside the container: the whole container).
pub fn container_pss_kb() -> u64 {
    let Ok(rd) = std::fs::read_dir("/proc") else { return 0 };
    rd.flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.bytes().all(|b| b.is_ascii_digit()).then(|| proc_mem(&n)).flatten()
        })
        .map(|m| m.pss_kb)
        .sum()
}
