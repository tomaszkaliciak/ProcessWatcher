use libc;

use crossterm::event::{self, KeyCode};
use ratatui::layout::{Constraint, Direction, Layout, Rows};
use ratatui::style::Color;
use ratatui::symbols;
use ratatui::{
    DefaultTerminal, Frame,
    style::{Modifier, Style, Stylize},
    symbols::border,
    text::{Line, Span, Text},
    widgets::{Block, Gauge, LineGauge, List, ListItem, Paragraph, Row, Table, TableState, Widget},
};
use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::mem;
use tokio::io::AsyncReadExt;
use tokio::runtime::Builder;
use tokio::sync::mpsc;
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

pub struct InfoReceiver {
    reciver: mpsc::Receiver<MemInfo>,
}

fn main() -> io::Result<()> {
    ratatui::run(|terminal| App::default().run(terminal))
}

#[derive(Debug, Default)]
pub struct App {
    totalram: u64,
    freeram: u64,
    cpu_usage: BTreeMap<String, f32>,
    proc_info: Vec<ProcStatus>,
    exit: bool,
}

impl App {
    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        let mut info_receiver = InfoReceiver::new();

        let mut table_state = TableState::default();
        table_state.select_first();
        table_state.select_first_column();

        while !self.exit {
            terminal.draw(|frame| self.draw(frame, &mut table_state))?;

            self.handle_events(&mut table_state);

            while let Ok(result) = info_receiver.reciver.try_recv() {
                self.freeram = result.free_memory;
                self.totalram = result.total_memory;
                self.proc_info = result.process_stats;
                self.cpu_usage = result.cpu_usage;
            }
        }
        Ok(())
    }

    fn draw(&self, frame: &mut Frame, table_state: &mut TableState) {
        let title = Line::from(" Process Watcher ".bold());
        let instructions = Line::from(vec![
            " Update ".into(),
            "<u>".blue().bold(),
            " Quit ".into(),
            "<Q> ".blue().bold(),
        ]);
        let block = Block::bordered()
            .title(title.centered())
            .title_bottom(instructions.centered())
            .border_set(border::THICK);

        let counter_text = Text::from(vec![Line::from(vec![
            "Total memory (KB): ".into(),
            self.totalram.to_string().yellow(),
            " Free memory (KB): ".into(),
            self.freeram.to_string().green(),
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

        for proc_info in &self.proc_info {
            rows.push(Row::new([
                proc_info.name.clone(),
                proc_info.pid.to_string(),
                proc_info.vm_size.to_string(),
                proc_info.vm_rss.to_string(),
                proc_info.rss_shem.to_string(),
                proc_info.rss_proc.to_string(),
                proc_info.cpu_usage.to_string(),
            ]));
        }

        let constraints_cpu_usage: Vec<Constraint> = self
            .cpu_usage
            .iter()
            .map(|_| Constraint::Length(1))
            .collect();

        let chunks_cpu = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints_cpu_usage)
            .split(chunks[2]);

        for (i, entry) in self.cpu_usage.iter().enumerate() {
            let cpu_usage_procent = entry.1 / 100.0;
            let line_gauge = LineGauge::default()
                .filled_style(Style::new().white().on_red().bold())
                .unfilled_style(Style::new().gray().on_black())
                .label(String::from(entry.0) + " " + cpu_usage_procent.to_string().as_str())
                .ratio((entry.1 / 100.0) as f64);

            frame.render_widget(line_gauge, chunks_cpu[i]);
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
                        KeyCode::Char('u') => self.update(),
                        KeyCode::Down => table_state.select_next(),
                        KeyCode::Up => table_state.select_previous(),
                        KeyCode::Right => table_state.select_next_column(),
                        KeyCode::Left => table_state.select_previous_column(),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn update(&mut self) {
        unsafe {
            let mut info: libc::sysinfo = mem::zeroed();

            if libc::sysinfo(&mut info) == 0 {
                self.freeram = info.freeram;
                self.totalram = info.totalram;
            } else {
                eprintln!("Failed to get system info.");
            }
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
    pub fn new() -> InfoReceiver {
        let (send, recv) = mpsc::channel(10);

        let rt = Builder::new_current_thread().enable_all().build().unwrap();

        _ = std::thread::Builder::new()
            .name("sysinfo-worker".to_string())
            .spawn(move || {
                rt.block_on(async move {
                        let task = tokio::spawn(async move {
                            let mut cpu_proc_stat_cache: HashMap<i32, ProcessCpuTime> =
                                HashMap::new();

                            let mut last_sample_time = time::Instant::now();

                            let mut cpu_usage_state: HashMap<String, CpuUsageState> =
                                HashMap::new();

                            let mut interval = time::interval(Duration::from_secs(2));
                            loop {
                                interval.tick().await;

                                let mut mem_info: MemInfo = MemInfo::default();

                                unsafe {
                                    let mut info: libc::sysinfo = mem::zeroed();

                                    if libc::sysinfo(&mut info) == 0 {
                                        mem_info.free_memory = info.freeram;
                                        mem_info.total_memory = info.totalram;
                                    } else {
                                        eprintln!("Failed to get system info.");
                                    }
                                }

                                let paths = fs::read_dir("/proc").unwrap();

                                let mut proc_stats = Vec::new();

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
                                        let status_path = "/proc/".to_string()
                                            + pid.to_string().as_str()
                                            + "/status";

                                        let mut contents = Vec::new();

                                        let mut statm_result = ProcStatus::default();

                                        if let Ok(file) =
                                            tokio::fs::File::open(status_path).await.as_mut()
                                        {
                                            let _ = file.read_to_end(&mut contents).await;

                                            let output = String::from_utf8(contents).unwrap();

                                            for line in output.lines() {
                                                if let Some(matching) = line.strip_prefix("Name:") {
                                                    statm_result.name =
                                                        matching.trim_start().to_string();
                                                } else if let Some(matching) =
                                                    line.strip_prefix("Pid:")
                                                {
                                                    if let Ok(parsed_pid) =
                                                        matching.trim_start().parse()
                                                    {
                                                        statm_result.pid = parsed_pid;
                                                    }
                                                } else if let Some(matching) =
                                                    line.strip_prefix("VmSize:")
                                                {
                                                    if let Some(digit_part) = matching
                                                        .trim_start()
                                                        .split_whitespace()
                                                        .next()
                                                    {
                                                        if let Ok(parsed) =
                                                            digit_part.parse::<u64>()
                                                        {
                                                            statm_result.vm_size = parsed;
                                                        }
                                                    }
                                                } else if let Some(matching) =
                                                    line.strip_prefix("VmRSS:")
                                                {
                                                    if let Some(digit_part) = matching
                                                        .trim_start()
                                                        .split_whitespace()
                                                        .next()
                                                    {
                                                        if let Ok(parsed) =
                                                            digit_part.parse::<u64>()
                                                        {
                                                            statm_result.vm_rss = parsed;
                                                            statm_result.rss_proc = (parsed * 1024)
                                                                as f32
                                                                / (mem_info.total_memory as f32)
                                                                * 100.0;
                                                        }
                                                    }
                                                } else if let Some(matching) =
                                                    line.strip_prefix("RssShmem:")
                                                {
                                                    if let Some(digit_part) = matching
                                                        .trim_start()
                                                        .split_whitespace()
                                                        .next()
                                                    {
                                                        if let Ok(parsed) =
                                                            digit_part.parse::<u64>()
                                                        {
                                                            statm_result.rss_shem = parsed;
                                                        }
                                                    }
                                                    break;
                                                }
                                            }
                                        }

                                        let mut contents_2 = Vec::new();

                                        let stat_path = "/proc/".to_string()
                                            + pid.to_string().as_str()
                                            + "/stat";

                                        if let Ok(file) =
                                            tokio::fs::File::open(stat_path).await.as_mut()
                                        {
                                            let _ = file.read_to_end(&mut contents_2).await;

                                            let output = String::from_utf8(contents_2).unwrap();

                                            let splits: Vec<&str> =
                                                output.split_whitespace().collect();

                                            if let (Some(user_time), Some(system_time)) =
                                                { (splits.get(13), splits.get(14)) }
                                            {
                                                if let (
                                                    Ok(parsed_user_time),
                                                    Ok(parsed_system_time),
                                                ) = {
                                                    (
                                                        user_time.parse::<u64>(),
                                                        system_time.parse::<u64>(),
                                                    )
                                                } {
                                                    if let Some(entry) =
                                                        cpu_proc_stat_cache.get(&pid)
                                                    {
                                                        let nb_processors = 8;

                                                        let delta_user = parsed_user_time
                                                            .saturating_sub(entry.user_time);
                                                        let delta_system = parsed_system_time
                                                            .saturating_sub(entry.system_time);

                                                        let ticks =
                                                            (delta_user + delta_system) as f32;

                                                        let elapsed_seconds = last_sample_time
                                                            .elapsed()
                                                            .as_secs_f32();

                                                        let cpu_usage = (ticks
                                                            * nb_processors as f32)
                                                            / (100.0 * elapsed_seconds);

                                                        statm_result.cpu_usage = cpu_usage;
                                                        let _ = cpu_proc_stat_cache.insert(
                                                            pid,
                                                            ProcessCpuTime {
                                                                user_time: parsed_user_time,
                                                                system_time: parsed_system_time,
                                                            },
                                                        );
                                                    } else {
                                                        cpu_proc_stat_cache.insert(
                                                            pid,
                                                            ProcessCpuTime {
                                                                user_time: parsed_user_time,
                                                                system_time: parsed_system_time,
                                                            },
                                                        );
                                                    }
                                                }
                                            }
                                        }

                                        if !statm_result.name.is_empty() && statm_result.vm_size > 0
                                        {
                                            proc_stats.push(statm_result);
                                        }
                                    }
                                }

                                if let Ok(file) = tokio::fs::File::open("/proc/stat").await.as_mut()
                                {
                                    let mut contents = Vec::new();
                                    let _ = file.read_to_end(&mut contents).await;

                                    let output2 = String::from_utf8(contents).unwrap();

                                    for line in output2.lines() {
                                        if !line.starts_with("cpu") {
                                            break;
                                        }

                                        let splits: Vec<&str> = line.split_whitespace().collect();

                                        if let (
                                            Some(r_cpu_id),
                                            Some(r_user),
                                            Some(r_nice),
                                            Some(r_system),
                                            Some(r_idle),
                                            Some(r_iowait),
                                            Some(r_irq),
                                            Some(r_softirq),
                                            Some(r_steal),
                                        ) = {
                                            (
                                                splits.get(0),
                                                splits.get(1),
                                                splits.get(2),
                                                splits.get(3),
                                                splits.get(4),
                                                splits.get(5),
                                                splits.get(6),
                                                splits.get(7),
                                                splits.get(8),
                                            )
                                        } {
                                            if let (
                                                Ok(user),
                                                Ok(nice),
                                                Ok(system),
                                                Ok(idle),
                                                Ok(iowait),
                                                Ok(irq),
                                                Ok(softirq),
                                                Ok(steal),
                                            ) = {
                                                (
                                                    r_user.parse::<u64>(),
                                                    r_nice.parse::<u64>(),
                                                    r_system.parse::<u64>(),
                                                    r_idle.parse::<u64>(),
                                                    r_iowait.parse::<u64>(),
                                                    r_irq.parse::<u64>(),
                                                    r_softirq.parse::<u64>(),
                                                    r_steal.parse::<u64>(),
                                                )
                                            } {
                                                let total_idle = idle + iowait;
                                                let total = user
                                                    + nice
                                                    + system
                                                    + total_idle
                                                    + irq
                                                    + softirq
                                                    + steal;
                                                let work_time = total - total_idle;

                                                if let Some(old_cpu_usage) = cpu_usage_state
                                                    .get_mut(r_cpu_id.to_owned())
                                                {
                                                    let cpu_usage = ((work_time
                                                        - old_cpu_usage.work_time)
                                                        as f32
                                                        / (total - old_cpu_usage.total_time)
                                                            as f32)
                                                        * 100.0;
                                                    mem_info
                                                        .cpu_usage
                                                        .insert(r_cpu_id.to_string(), cpu_usage);

                                                    old_cpu_usage.total_time = total;
                                                    old_cpu_usage.work_time = work_time;
                                                } else {
                                                    cpu_usage_state.insert(
                                                        r_cpu_id.to_string(),
                                                        CpuUsageState {
                                                            work_time: 0,
                                                            total_time: 0,
                                                        },
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }

                                mem_info.process_stats = proc_stats;
                                mem_info.process_stats.sort_by_key(|u| Reverse(u.vm_rss));
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
