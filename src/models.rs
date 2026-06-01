use std::collections::VecDeque;

#[derive(Debug, PartialEq, Default)]
pub struct RingBuffer<T> {
    pub buf: VecDeque<T>,
    pub capacity: usize,
}

impl<T> RingBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, item: T) {
        if self.buf.len() == self.capacity {
            self.buf.pop_back();
        }
        self.buf.push_front(item);
    }
}

#[derive(Debug, PartialEq)]
pub struct ProcessHistory {
    pub pid: u64,
    pub history: RingBuffer<ProcessStatus>,
}

#[derive(Debug, PartialEq, Default)]
pub struct MemCpuHistory {
    pub history: RingBuffer<MemCpuInfo>,
}

#[derive(Debug, PartialEq, Default)]
pub struct MemCpuInfo {
    pub total_memory: u64,
    pub available_memory: u64,
    pub free_memory: u64,
    pub cpu_usage: std::collections::BTreeMap<String, f32>,
}

#[derive(Debug, Default)]
pub struct MemInfo {
    pub mem_cpu_stats: MemCpuInfo,
    pub process_stats: Vec<ProcessInfo>,
}

#[derive(Debug, Default, Clone)]
pub struct ProcessInfo {
    pub command: String,
    pub pid: u64,
    pub parent_pid: Option<u64>,
    pub status: ProcessStatus,
}

#[derive(Debug, Default, PartialEq, Clone, Copy)]
pub struct ProcessStatus {
    pub vm_size: u64,
    pub vm_rss: u64,
    pub rss_shem: u64,
    pub rss_proc: f32,
    pub cpu_usage: f32,
}

pub struct ProcessCpuTime {
    pub user_time: u64,
    pub system_time: u64,
}

pub struct CpuUsageState {
    pub work_time: u64,
    pub total_time: u64,
}
