use crate::models::{CpuUsageState, MemInfo, ProcessCpuTime, ProcessInfo};
use tokio::task::JoinSet;

use crate::parser::{
    get_free_available_total_memory, get_proc_stat_data, get_proc_uptime, parse_proc_pid_parent,
    parse_proc_pid_stat_cpu_usage, parse_proc_pid_stat_uptime, parse_proc_pid_status,
};
use std::collections::HashMap;
use std::fs;
use tokio::runtime::Builder;
use tokio::sync::mpsc;
use tokio::time::{self, Duration};

pub struct InfoReceiver {
    pub reciver: mpsc::Receiver<MemInfo>,
}

impl InfoReceiver {
    pub fn new() -> InfoReceiver {
        let (send, recv) = mpsc::channel(10);

        let rt = Builder::new_current_thread().enable_all().build().unwrap();

        _ = std::thread::Builder::new()
            .name("sysinfo-worker".to_string())
            .spawn(move || {
                rt.block_on(async move {
                    let task = tokio::spawn(async move {
                        let mut last_sample_time = time::Instant::now();

                        let mut cpu_usage_state: HashMap<String, CpuUsageState> = HashMap::new();

                        let mut cpu_proc_stat_cache: HashMap<i32, ProcessCpuTime> = HashMap::new();

                        let mut interval = time::interval(Duration::from_secs(2));
                        loop {
                            interval.tick().await;

                            let mut mem_info: MemInfo = MemInfo::default();

                            let meminfo_output =
                                tokio::fs::read_to_string("/proc/meminfo").await.unwrap();
                            (
                                mem_info.mem_cpu_stats.free_memory,
                                mem_info.mem_cpu_stats.available_memory,
                                mem_info.mem_cpu_stats.total_memory,
                            ) = get_free_available_total_memory(meminfo_output);

                            let mut proc_stats = Vec::new();

                            let elapsed_seconds = last_sample_time.elapsed().as_secs_f32();

                            let mut set = JoinSet::new();

                            let system_uptime = get_proc_uptime().await;

                            for path in fs::read_dir("/proc").unwrap() {
                                let pid_result: Result<i32, _> = path
                                    .unwrap()
                                    .path()
                                    .file_name()
                                    .unwrap()
                                    .to_str()
                                    .unwrap()
                                    .parse();

                                if let Ok(pid) = pid_result {
                                    set.spawn(async move {
                                        let (stat, status, cmd) = tokio::join!(
                                            tokio::fs::read_to_string(format!(
                                                "/proc/{}/stat",
                                                pid
                                            )),
                                            tokio::fs::read_to_string(format!(
                                                "/proc/{}/status",
                                                pid
                                            )),
                                            tokio::fs::read_to_string(format!(
                                                "/proc/{}/cmdline",
                                                pid
                                            ))
                                        );
                                        Ok::<_, std::io::Error>((pid, stat, status, cmd))
                                    });
                                }
                            }

                            while let Some(res) = set.join_next().await {
                                if let Ok(Ok((pid, Ok(stat), Ok(status), Ok(cmd)))) = res {
                                    let parent_pid = parse_proc_pid_parent(&status);

                                    let mut status_result = parse_proc_pid_status(
                                        status,
                                        mem_info.mem_cpu_stats.total_memory,
                                    );

                                    if status_result.vm_size == 0 {
                                        continue;
                                    }

                                    status_result.uptime =
                                        parse_proc_pid_stat_uptime(system_uptime, &stat);

                                    status_result.cpu_usage = parse_proc_pid_stat_cpu_usage(
                                        pid,
                                        stat,
                                        elapsed_seconds,
                                        &mut cpu_proc_stat_cache,
                                    );

                                    let process_info = ProcessInfo {
                                        command: cmd,
                                        pid: pid as u64,
                                        parent_pid,
                                        status: status_result,
                                        pid_lvl: 0,
                                    };

                                    proc_stats.push(process_info);
                                }
                            }

                            mem_info.uptime = system_uptime;
                            mem_info.mem_cpu_stats.cpu_usage =
                                get_proc_stat_data(&mut cpu_usage_state).await;

                            mem_info.process_stats = proc_stats;
                            send.send(mem_info).await.unwrap();
                            last_sample_time = time::Instant::now();
                        }
                    });
                    task.await.unwrap();
                });
            });

        InfoReceiver { reciver: recv }
    }
}
