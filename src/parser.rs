use crate::models::{CpuUsageState, ProcessCpuTime, ProcessStatus};
use libc;
use std::collections::{BTreeMap, HashMap};
use std::mem;
use tokio::io::AsyncReadExt;

pub fn get_free_and_total_memory() -> (u64, u64) {
    unsafe {
        let mut info: libc::sysinfo = mem::zeroed();
        if libc::sysinfo(&mut info) == 0 {
            return (info.freeram, info.totalram);
        } else {
            eprintln!("Failed to get system info.");
        }
    }
    (0, 0)
}

pub fn parse_proc_pid_status(status_str: String, total_memory: u64) -> ProcessStatus {
    let mut statm_result = ProcessStatus::default();
    for line in status_str.lines() {
        if let Some(matching) = line.strip_prefix("VmSize:") {
            if let Some(digit_part) = matching.trim_start().split_whitespace().next() {
                if let Ok(parsed) = digit_part.parse::<u64>() {
                    statm_result.vm_size = parsed;
                }
            }
        } else if let Some(matching) = line.strip_prefix("VmRSS:") {
            if let Some(digit_part) = matching.trim_start().split_whitespace().next() {
                if let Ok(parsed) = digit_part.parse::<u64>() {
                    statm_result.vm_rss = parsed;
                    statm_result.rss_proc = (parsed * 1024) as f32 / (total_memory as f32) * 100.0;
                }
            }
        } else if let Some(matching) = line.strip_prefix("RssShmem:") {
            if let Some(digit_part) = matching.trim_start().split_whitespace().next() {
                if let Ok(parsed) = digit_part.parse::<u64>() {
                    statm_result.rss_shem = parsed;
                }
            }
            break;
        }
    }
    statm_result
}

pub fn parse_proc_pid_stat_cpu_usage(
    pid_id: i32,
    input: String,
    elapsed_seconds: f32,
    cpu_proc_stat_cache: &mut HashMap<i32, ProcessCpuTime>,
) -> f32 {
    let splits: Vec<&str> = input.split_whitespace().collect();

    if let (Some(user_time), Some(system_time)) = { (splits.get(13), splits.get(14)) } {
        if let (Ok(parsed_user_time), Ok(parsed_system_time)) =
            { (user_time.parse::<u64>(), system_time.parse::<u64>()) }
        {
            if let Some(entry) = cpu_proc_stat_cache.get(&pid_id) {
                let nb_processors = 8;

                let delta_user = parsed_user_time.saturating_sub(entry.user_time);
                let delta_system = parsed_system_time.saturating_sub(entry.system_time);

                let ticks = (delta_user + delta_system) as f32;

                let cpu_usage = (ticks * nb_processors as f32) / (100.0 * elapsed_seconds);

                let _ = cpu_proc_stat_cache.insert(
                    pid_id,
                    ProcessCpuTime {
                        user_time: parsed_user_time,
                        system_time: parsed_system_time,
                    },
                );
                return cpu_usage;
            } else {
                cpu_proc_stat_cache.insert(
                    pid_id,
                    ProcessCpuTime {
                        user_time: parsed_user_time,
                        system_time: parsed_system_time,
                    },
                );
            }
        }
    }

    return 0.0;
}

pub async fn get_proc_stat_data(
    cpu_usage_state_cache: &mut HashMap<String, CpuUsageState>,
) -> BTreeMap<String, f32> {
    let mut output: BTreeMap<String, f32> = BTreeMap::new();

    if let Ok(file) = tokio::fs::File::open("/proc/stat").await.as_mut() {
        let mut contents = Vec::new();
        let _ = file.read_to_end(&mut contents).await;

        let output2 = String::from_utf8(contents).unwrap();

        for line in output2.lines() {
            if !line.starts_with("cpu") {
                return output;
            }

            if let Ok(cpu_times) = parse_cpu_times(&line) {
                let total_idle_time = cpu_times.idle + cpu_times.iowait;
                let total_time = cpu_times.user
                    + cpu_times.nice
                    + cpu_times.system
                    + total_idle_time
                    + cpu_times.irq
                    + cpu_times.softirq
                    + cpu_times.steal;
                let work_time = total_time - total_idle_time;

                if let Some(old_cpu_usage) = cpu_usage_state_cache.get_mut(&cpu_times.cpu_name) {
                    let cpu_usage = ((work_time - old_cpu_usage.work_time) as f32
                        / (total_time - old_cpu_usage.total_time) as f32)
                        * 100.0;
                    output.insert(cpu_times.cpu_name, cpu_usage);

                    old_cpu_usage.total_time = total_time;
                    old_cpu_usage.work_time = work_time;
                } else {
                    cpu_usage_state_cache.insert(
                        cpu_times.cpu_name.to_string(),
                        CpuUsageState {
                            work_time: 0,
                            total_time: 0,
                        },
                    );
                }
            }
        }
    }

    return output;
}

#[derive(Debug, Default)]
pub struct CpuTimes {
    pub cpu_name: String,
    pub user: u64,
    pub nice: u64,
    pub system: u64,
    pub idle: u64,
    pub iowait: u64,
    pub irq: u64,
    pub softirq: u64,
    pub steal: u64,
}

pub fn parse_cpu_times(line: &str) -> Result<CpuTimes, std::num::ParseIntError> {
    let mut iter = line.split_whitespace();
    Ok(CpuTimes {
        cpu_name: iter.next().unwrap().to_owned(),
        user: iter.next().unwrap().parse()?,
        nice: iter.next().unwrap().parse()?,
        system: iter.next().unwrap().parse()?,
        idle: iter.next().unwrap().parse()?,
        iowait: iter.next().unwrap().parse()?,
        irq: iter.next().unwrap().parse()?,
        softirq: iter.next().unwrap().parse()?,
        steal: iter.next().unwrap().parse()?,
    })
}
