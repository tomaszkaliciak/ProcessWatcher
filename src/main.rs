use libc;

use crossterm::event::{self, KeyCode};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Color;
use ratatui::symbols;
use ratatui::{
    DefaultTerminal, Frame,
    style::{Modifier, Style, Stylize},
    symbols::border,
    text::{Line, Span, Text},
    widgets::{Block, Gauge, List, ListItem, Paragraph, Row, Table, TableState},
};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::mem;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::runtime::Builder;
use tokio::sync::Mutex;

use tokio::time::{self, Duration};

#[derive(Debug, Default)]
pub struct MemInfo {
    total_memory: u64,
    free_memory: u64,
    cpu_usage: BTreeMap<String, f32>,
    process_stats: Vec<ProcStatus>,
}

impl MemInfo {
    pub fn send(
        state: (u64, u64),
        cpu_usage: BTreeMap<String, f32>,
        proc_stats: Vec<ProcStatus>,
    ) -> MemInfo {
        MemInfo {
            total_memory: state.0,
            free_memory: state.1,
            cpu_usage: cpu_usage,
            process_stats: proc_stats,
        }
    }
}

pub struct InfoReceiver {}

fn main() -> io::Result<()> {
    ratatui::run(|terminal| App::default().run(terminal))
}

const UI_COLUMNS: [ProcessUiColumn; 7] = [
    ProcessUiColumn::Name,
    ProcessUiColumn::Pid,
    ProcessUiColumn::VmSize,
    ProcessUiColumn::VmRss,
    ProcessUiColumn::RssShem,
    ProcessUiColumn::RssProc,
    ProcessUiColumn::CpuUsage,
];

#[derive(Debug, Default, PartialEq, Clone, Copy)]
enum ProcessUiColumn {
    Name,
    Pid,
    VmSize,
    #[default]
    VmRss,
    RssShem,
    RssProc,
    CpuUsage,
}

#[derive(Debug, Default, PartialEq)]
enum SortOrder {
    Ascending,
    #[default]
    Descending,
}

#[derive(Debug, Default, PartialEq)]
struct TableOrderSettings {
    order_by_field: ProcessUiColumn,
    order: SortOrder,
}

#[derive(Debug, Default)]
pub struct App {
    table_order_settings: TableOrderSettings,
    exit: bool,
}

impl App {
    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        let mut table_state = TableState::default();
        table_state.select_first();
        table_state.select_first_column();

        let data = Arc::new(Mutex::new(MemInfo::default()));

        let _ = InfoReceiver::new(data.clone());

        while !self.exit {
            if let Ok(mut data_locked) = data.try_lock() {
                terminal.draw(|frame| self.draw(frame, &mut table_state, &mut data_locked))?;
            }

            self.handle_events(&mut table_state);
        }
        Ok(())
    }

    fn sort(&self, mem_info: &mut MemInfo) {
        match self.table_order_settings.order_by_field {
            ProcessUiColumn::Name => mem_info.process_stats.sort_by(|a, b| a.name.cmp(&b.name)),
            ProcessUiColumn::Pid => mem_info.process_stats.sort_by_key(|u| u.pid),
            ProcessUiColumn::VmSize => mem_info.process_stats.sort_by_key(|u| u.vm_size),
            ProcessUiColumn::VmRss => mem_info.process_stats.sort_by_key(|u| u.vm_rss),
            ProcessUiColumn::RssShem => mem_info.process_stats.sort_by_key(|u| u.rss_shem),
            ProcessUiColumn::RssProc => mem_info
                .process_stats
                .sort_by(|u, v| u.rss_proc.total_cmp(&v.rss_proc)),
            ProcessUiColumn::CpuUsage => mem_info
                .process_stats
                .sort_by(|u, v| u.cpu_usage.total_cmp(&v.cpu_usage)),
        }

        if self.table_order_settings.order == SortOrder::Descending {
            mem_info.process_stats.reverse();
        }
    }

    fn draw(&self, frame: &mut Frame, table_state: &mut TableState, data: &mut MemInfo) {
        self.sort(data);
        let title = Line::from(" Process Watcher ".bold());
        let instructions = Line::from(vec![" Quit ".into(), "<Q> ".blue().bold()]);
        let block = Block::bordered()
            .title(title.centered())
            .title_bottom(instructions.centered())
            .border_set(border::THICK);

        let counter_text = Text::from(vec![Line::from(vec![
            "Total memory (KB): ".into(),
            data.total_memory.to_string().yellow(),
            " Free memory (KB): ".into(),
            data.free_memory.to_string().green(),
        ])]);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(1),
                Constraint::Length(8),
            ])
            .spacing(1)
            .split(frame.area());

        let paragraph = Paragraph::new(counter_text).centered().block(block);

        frame.render_widget(paragraph, chunks[0]);

        let header = Row::new(["name", "PID", "VIRT", "RSS", "SHR", "MEM(%)", "CPU(%)"])
            .style(Style::new().bold())
            .bottom_margin(1);

        let mut rows: Vec<Row> = Vec::new();

        for proc_info in &data.process_stats {
            rows.push(Row::new([
                proc_info.name.clone(),
                proc_info.pid.to_string(),
                proc_info.vm_size.to_string(),
                proc_info.vm_rss.to_string(),
                proc_info.rss_shem.to_string(),
                proc_info.rss_proc.to_string(),
                format!("{:>2.4}", proc_info.cpu_usage.to_string()),
            ]));
        }

        let items: Vec<ListItem> = data
            .cpu_usage
            .keys()
            .map(|key| {
                ListItem::new(Line::from(vec![
                    Span::raw(symbols::DOT),
                    Span::styled(
                        format!("{}", key),
                        Style::default()
                            .fg(Color::LightGreen)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]))
            })
            .collect();

        let list = List::new(items);
        frame.render_widget(list, chunks[2]);

        let chunk = chunks[2];
        let max_rows = chunk.height as usize;

        for (i, (_, usage)) in data.cpu_usage.iter().take(max_rows).enumerate() {
            let y = chunk.top() + i as u16;

            let gauge = Gauge::default()
                .gauge_style(Style::default().fg(Color::Yellow))
                .ratio((usage / 100.0) as f64);

            frame.render_widget(
                gauge,
                Rect {
                    x: chunk.left() + 10,
                    y,
                    width: chunk.width.saturating_sub(10),
                    height: 1,
                },
            );
        }

        let widths = [
            Constraint::Percentage(10),
            Constraint::Percentage(10),
            Constraint::Percentage(10),
            Constraint::Percentage(10),
            Constraint::Percentage(10),
            Constraint::Percentage(10),
            Constraint::Percentage(10),
        ];
        let table = Table::new(rows, widths)
            .header(header)
            .column_spacing(1)
            .style(Color::White)
            .row_highlight_style(Style::new().on_black().bold())
            .column_highlight_style(Color::Gray)
            .cell_highlight_style(Style::new().reversed().yellow())
            .highlight_symbol("() ");

        frame.render_stateful_widget(table, chunks[1], table_state);
    }

    fn handle_events(&mut self, table_state: &mut TableState) {
        match event::poll(Duration::from_millis(100)) {
            Ok(true) => {
                if let Ok(event::Event::Key(key)) = event::read() {
                    match key.code {
                        KeyCode::Char('q') => self.exit(),
                        KeyCode::Down => table_state.select_next(),
                        KeyCode::Up => table_state.select_previous(),
                        KeyCode::Right => table_state.select_next_column(),
                        KeyCode::Left => table_state.select_previous_column(),
                        KeyCode::Char('s') => {
                            if let Some(idx) = table_state.selected_column() {
                                if let Some(col) = UI_COLUMNS.get(idx) {
                                    if self.table_order_settings.order_by_field != *col {
                                        self.table_order_settings.order_by_field = *col;
                                        self.table_order_settings.order = SortOrder::Descending;
                                    } else {
                                        if self.table_order_settings.order == SortOrder::Ascending {
                                            self.table_order_settings.order = SortOrder::Descending;
                                        } else {
                                            self.table_order_settings.order = SortOrder::Ascending;
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn exit(&mut self) {
        self.exit = true;
    }
}

struct ProcessCpuTime {
    user_time: u64,
    system_time: u64,
}

struct CpuUsageState {
    work_time: u64,
    total_time: u64,
}

impl InfoReceiver {
    pub fn new(m_mem_info: Arc<Mutex<MemInfo>>) -> InfoReceiver {
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

                            let mut mem_info = m_mem_info.lock().await;

                            mem_info.process_stats.clear();
                            (mem_info.free_memory, mem_info.total_memory) =
                                get_free_and_total_memory();

                            let paths = fs::read_dir("/proc").unwrap();

                            let mut proc_stats = Vec::new();

                            let elapsed_seconds = last_sample_time.elapsed().as_secs_f32();

                            for path in paths {
                                let pid_result: Result<i32, _> = path
                                    .unwrap()
                                    .path()
                                    .file_name()
                                    .unwrap()
                                    .to_str()
                                    .unwrap()
                                    .parse();

                                if let Ok(pid) = pid_result {
                                    let mut statm_result =
                                        get_proc_pid_status(pid, mem_info.total_memory).await;

                                    statm_result.cpu_usage = get_proc_pid_stat_cpu_usage(
                                        pid,
                                        elapsed_seconds,
                                        &mut cpu_proc_stat_cache,
                                    )
                                    .await;

                                    if !statm_result.name.is_empty() && statm_result.vm_size > 0 {
                                        proc_stats.push(statm_result);
                                    }
                                }
                            }

                            mem_info.cpu_usage = get_proc_stat_data(&mut cpu_usage_state).await;

                            mem_info.process_stats = proc_stats;
                            last_sample_time = time::Instant::now();
                        }
                    });
                    task.await.unwrap();
                });
            });

        InfoReceiver {}
    }
}

#[derive(Debug, Default)]
pub struct ProcStatus {
    pub name: String,
    pub pid: u64,
    pub vm_size: u64,
    pub vm_rss: u64,
    pub rss_shem: u64,
    pub rss_proc: f32,
    pub cpu_usage: f32,
}

fn get_free_and_total_memory() -> (u64, u64) {
    unsafe {
        let mut info: libc::sysinfo = mem::zeroed();

        if libc::sysinfo(&mut info) == 0 {
            return (info.freeram, info.totalram);
        } else {
            eprintln!("Failed to get system info.");
        }
    }

    return (0, 0);
}

async fn get_proc_pid_stat_cpu_usage(
    pid_id: i32,
    elapsed_seconds: f32,
    cpu_proc_stat_cache: &mut HashMap<i32, ProcessCpuTime>,
) -> f32 {
    let mut contents_2 = Vec::new();

    let stat_path = "/proc/".to_string() + pid_id.to_string().as_str() + "/stat";

    if let Ok(file) = tokio::fs::File::open(stat_path).await.as_mut() {
        let _ = file.read_to_end(&mut contents_2).await;

        let output = String::from_utf8(contents_2).unwrap();

        let splits: Vec<&str> = output.split_whitespace().collect();

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
    }

    return 0.0;
}

async fn get_proc_pid_status(pid_id: i32, total_memory: u64) -> ProcStatus {
    let mut statm_result = ProcStatus::default();

    let status_path = "/proc/".to_string() + pid_id.to_string().as_str() + "/status";

    let mut contents = Vec::new();

    if let Ok(file) = tokio::fs::File::open(status_path).await.as_mut() {
        let _ = file.read_to_end(&mut contents).await;

        let output = String::from_utf8(contents).unwrap();

        for line in output.lines() {
            if let Some(matching) = line.strip_prefix("Name:") {
                statm_result.name = matching.trim_start().to_string();
            } else if let Some(matching) = line.strip_prefix("Pid:") {
                if let Ok(parsed_pid) = matching.trim_start().parse() {
                    statm_result.pid = parsed_pid;
                }
            } else if let Some(matching) = line.strip_prefix("VmSize:") {
                if let Some(digit_part) = matching.trim_start().split_whitespace().next() {
                    if let Ok(parsed) = digit_part.parse::<u64>() {
                        statm_result.vm_size = parsed;
                    }
                }
            } else if let Some(matching) = line.strip_prefix("VmRSS:") {
                if let Some(digit_part) = matching.trim_start().split_whitespace().next() {
                    if let Ok(parsed) = digit_part.parse::<u64>() {
                        statm_result.vm_rss = parsed;
                        statm_result.rss_proc =
                            (parsed * 1024) as f32 / (total_memory as f32) * 100.0;
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
    }

    return statm_result;
}

async fn get_proc_stat_data(
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
struct CpuTimes {
    cpu_name: String,
    user: u64,
    nice: u64,
    system: u64,
    idle: u64,
    iowait: u64,
    irq: u64,
    softirq: u64,
    steal: u64,
}

fn parse_cpu_times(line: &str) -> Result<CpuTimes, std::num::ParseIntError> {
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
