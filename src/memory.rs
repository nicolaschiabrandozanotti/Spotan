//! Linux process accounting, including proportionally attributed swapped pages.
use std::{collections::BTreeMap, fs, path::Path};

#[derive(Debug)]
pub struct Usage {
    pub pid: u32,
    pub name: String,
    pub resident_kib: u64,
    pub swap_kib: u64,
}

fn values(data: &str) -> (u64, u64) {
    let value = |key: &str| {
        data.lines()
            .find_map(|line| {
                let (name, number) = line.split_once(':')?;
                (name == key)
                    .then(|| number.split_whitespace().next()?.parse::<u64>().ok())
                    .flatten()
            })
            .unwrap_or(0)
    };
    (value("Pss"), value("SwapPss"))
}
fn process(pid: u32) -> Option<(String, u32)> {
    let base = Path::new("/proc").join(pid.to_string());
    let name = fs::read_to_string(base.join("comm"))
        .ok()?
        .trim()
        .to_owned();
    let status = fs::read_to_string(base.join("status")).ok()?;
    let parent = status
        .lines()
        .find_map(|line| line.strip_prefix("PPid:"))?
        .trim()
        .parse()
        .ok()?;
    Some((name, parent))
}
pub fn usages() -> Vec<Usage> {
    let mut processes = BTreeMap::new();
    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|value| value.parse::<u32>().ok())
            else {
                continue;
            };
            let Some((name, mut parent)) = process(pid) else {
                continue;
            };
            let foot = name == "foot"
                && fs::read(format!("/proc/{pid}/cmdline"))
                    .ok()
                    .is_some_and(|args| {
                        args.windows(b"--app-id=ncspot".len())
                            .any(|part| part == b"--app-id=ncspot")
                    });
            if !foot && !matches!(name.as_str(), "ncspot" | "spoti-lyrics" | "spoti-popover") {
                continue;
            }
            processes.insert(pid, name);
            for _ in 0..12 {
                let Some((name, next)) = process(parent) else {
                    break;
                };
                if name == "kitty" || name == "foot" {
                    processes.insert(parent, name);
                    break;
                }
                if name.starts_with("tmux") {
                    processes.insert(parent, name);
                }
                if next == parent || parent <= 1 {
                    break;
                }
                parent = next;
            }
        }
    }
    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|value| value.parse::<u32>().ok())
            else {
                continue;
            };
            let Some((name, parent)) = process(pid) else {
                continue;
            };
            if name.starts_with("tmux") && processes.get(&parent).is_some_and(|name| name == "foot")
            {
                processes.insert(pid, name);
            }
        }
    }
    processes
        .into_iter()
        .filter_map(|(pid, name)| {
            let data = fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).ok()?;
            let (resident_kib, swap_kib) = values(&data);
            Some(Usage {
                pid,
                name,
                resident_kib,
                swap_kib,
            })
        })
        .collect()
}
pub fn print_report() {
    println!("Proceso          PID     RAM MiB  Swap MiB  Total MiB");
    let mut app = 0;
    let mut all = 0;
    let usages = usages();
    let native = usages.iter().any(|item| {
        item.name == "ncspot"
            && fs::read(format!("/proc/{}/cmdline", item.pid))
                .ok()
                .is_some_and(|args| args.split(|byte| *byte == 0).any(|arg| arg == b"--native"))
    });
    for item in usages {
        let total = item.resident_kib + item.swap_kib;
        if item.name != "kitty" && item.name != "foot" && !item.name.starts_with("tmux") {
            app += total;
        }
        all += total;
        println!(
            "{:<15} {:>7} {:>9.1} {:>9.1} {:>10.1}",
            item.name,
            item.pid,
            item.resident_kib as f64 / 1024.0,
            item.swap_kib as f64 / 1024.0,
            total as f64 / 1024.0
        );
    }
    if native && app == all {
        println!(
            "Total Spoti: {:.1} MiB · ventana nativa, sin terminal externa",
            all as f64 / 1024.0
        );
    } else {
        println!(
            "Spoti: {:.1} MiB · con terminal: {:.1} MiB",
            app as f64 / 1024.0,
            all as f64 / 1024.0
        );
    }
    println!("PSS + SwapPss: memoria compartida repartida; swap incluida para comparar.");
}

#[cfg(test)]
mod tests {
    #[test]
    fn counts_proportional_swap_without_using_rss_or_double_counting_shared_pages() {
        assert_eq!(
            super::values("Rss: 10000 kB\nPss: 3500 kB\nSwap: 2000 kB\nSwapPss: 500 kB\n"),
            (3500, 500)
        );
    }
}
