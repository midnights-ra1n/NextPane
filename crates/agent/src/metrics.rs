//! Host metrics from /proc, per-app metrics from each app's cgroup (cgroup v2).

use nextpane_proto::{AppMetrics, Metrics};
use std::{collections::HashMap, ffi::CString, fs, path::Path, sync::{Arc, Mutex}, time::{Instant, SystemTime, UNIX_EPOCH}};

#[derive(Default)]
pub struct Collector {
    cpu: Option<(u64, u64)>,
    net: Option<(u64, u64, Instant)>,
    app_cpu: HashMap<String, (u64, Instant)>,
    /// App disk usage, refreshed in the background every few minutes (walking node_modules is slow).
    pub disk: Arc<Mutex<HashMap<String, u64>>>,
}

fn read(p: &str) -> String {
    fs::read_to_string(p).unwrap_or_default()
}

/// (total, idle) jiffies from the aggregate line of /proc/stat.
fn parse_cpu(stat: &str) -> Option<(u64, u64)> {
    let v: Vec<u64> = stat.lines().next()?.split_whitespace().skip(1).take(8).filter_map(|x| x.parse().ok()).collect();
    (v.len() >= 5).then(|| (v.iter().sum(), v[3] + v[4]))
}

fn parse_meminfo(s: &str) -> HashMap<&str, u64> {
    s.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            Some((k, v.split_whitespace().next()?.parse::<u64>().ok()? * 1024))
        })
        .collect()
}

/// (rx, tx) bytes summed over all interfaces but loopback.
fn parse_net(s: &str) -> (u64, u64) {
    s.lines().skip(2).filter_map(|l| l.split_once(':')).filter(|(i, _)| i.trim() != "lo").fold((0, 0), |(rx, tx), (_, v)| {
        let f: Vec<u64> = v.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        (rx + f.first().unwrap_or(&0), tx + f.get(8).unwrap_or(&0))
    })
}

fn statvfs(path: &str) -> (u64, u64) {
    let Ok(c) = CString::new(path) else { return (0, 0) };
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return (0, 0);
    }
    let f = s.f_frsize as u64;
    ((s.f_blocks as u64 - s.f_bfree as u64) * f, s.f_blocks as u64 * f)
}

impl Collector {
    pub fn sample(&mut self, apps: &[String]) -> Metrics {
        let mut m = Metrics { ts: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()), ..Default::default() };

        if let Some((total, idle)) = parse_cpu(&read("/proc/stat")) {
            if let Some((pt, pi)) = self.cpu.replace((total, idle)) {
                let dt = total.saturating_sub(pt);
                if dt > 0 {
                    m.cpu = 100.0 * (1.0 - idle.saturating_sub(pi) as f32 / dt as f32);
                }
            }
        }

        let mi = read("/proc/meminfo");
        let mi = parse_meminfo(&mi);
        let g = |k: &str| mi.get(k).copied().unwrap_or(0);
        m.mem_total = g("MemTotal");
        m.mem_used = m.mem_total.saturating_sub(g("MemAvailable"));
        m.swap_total = g("SwapTotal");
        m.swap_used = m.swap_total.saturating_sub(g("SwapFree"));

        let load: Vec<f32> = read("/proc/loadavg").split_whitespace().take(3).filter_map(|x| x.parse().ok()).collect();
        if load.len() == 3 {
            m.load = [load[0], load[1], load[2]];
        }
        m.uptime = read("/proc/uptime").split('.').next().and_then(|s| s.parse().ok()).unwrap_or(0);

        let data = if Path::new(crate::apps::HOME).exists() { crate::apps::HOME } else { "/" };
        (m.disk_used, m.disk_total) = statvfs(data);

        let (rx, tx) = parse_net(&read("/proc/net/dev"));
        if let Some((prx, ptx, at)) = self.net.replace((rx, tx, Instant::now())) {
            let secs = at.elapsed().as_secs_f64().max(0.001);
            m.net_rx = (rx.saturating_sub(prx) as f64 / secs) as u64;
            m.net_tx = (tx.saturating_sub(ptx) as f64 / secs) as u64;
        }

        let cpus = std::thread::available_parallelism().map_or(1, |n| n.get()) as f32;
        let disk = self.disk.lock().unwrap().clone();
        self.app_cpu.retain(|k, _| apps.contains(k));
        for name in apps {
            let cg = format!("/sys/fs/cgroup/system.slice/nextpane-app-{name}.service");
            let usage: u64 = read(&format!("{cg}/cpu.stat")).lines().find_map(|l| l.strip_prefix("usage_usec ")?.trim().parse().ok()).unwrap_or(0);
            let cpu = match self.app_cpu.insert(name.clone(), (usage, Instant::now())) {
                Some((pu, at)) => usage.saturating_sub(pu) as f32 / at.elapsed().as_micros().max(1) as f32 * 100.0 / cpus,
                None => 0.0,
            };
            m.apps.push(AppMetrics {
                name: name.clone(),
                running: read(&format!("{cg}/cgroup.events")).contains("populated 1"),
                cpu,
                mem: read(&format!("{cg}/memory.current")).trim().parse().unwrap_or(0),
                disk: disk.get(name).copied().unwrap_or(0),
            });
        }
        m
    }
}

pub fn dir_size(p: &Path) -> u64 {
    let Ok(md) = fs::symlink_metadata(p) else { return 0 };
    if !md.is_dir() {
        return md.len();
    }
    fs::read_dir(p).map_or(0, |rd| rd.flatten().map(|e| dir_size(&e.path())).sum())
}

pub fn refresh_disk(apps: &[String], map: &Mutex<HashMap<String, u64>>) {
    let sizes: HashMap<String, u64> = apps.iter().map(|a| (a.clone(), dir_size(&crate::apps::app_dir(a)))).collect();
    *map.lock().unwrap() = sizes;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsers() {
        assert_eq!(parse_cpu("cpu  10 0 5 80 5 0 0 0 0 0\ncpu0 1 2"), Some((100, 85)));
        let mi = "MemTotal:  2048 kB\nMemAvailable: 1024 kB\n";
        assert_eq!(parse_meminfo(mi)["MemAvailable"], 1024 * 1024);
        let net = "h1\nh2\n    lo: 999 0 0 0 0 0 0 0 999 0\n  eth0: 100 1 0 0 0 0 0 0 200 2\n";
        assert_eq!(parse_net(net), (100, 200));
    }
}
