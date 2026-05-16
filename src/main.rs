use libc;

use crossterm::event::{self, KeyCode};
use ratatui::layout::{Constraint, Direction, Layout, Rows};
use ratatui::style::Color;
use ratatui::{
    DefaultTerminal, Frame,
    style::{Style, Stylize},
    symbols::border,
    text::{Line, Span, Text},
    widgets::{Block, List, ListItem, Paragraph, Row, Table, TableState, Widget},
};
use std::cmp::Reverse;
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
    process_stats: Vec<ProcStatus>,
}

impl MemInfo {
    pub fn send(state: (u64, u64), proc_stats: Vec<ProcStatus>) -> MemInfo {
        MemInfo {
            total_memory: state.0,
            free_memory: state.1,
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
            .constraints([Constraint::Length(3), Constraint::Min(1)])
            .spacing(1)
            .split(frame.area());

        let paragraph = Paragraph::new(counter_text).centered().block(block);

        frame.render_widget(paragraph, chunks[0]);

        let header = Row::new(["name", "PID", "VIRT", "RSS", "SHR"])
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
            ]));
        }

        let widths = [
            Constraint::Percentage(25),
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

impl InfoReceiver {
    pub fn new() -> InfoReceiver {
        let (send, recv) = mpsc::channel(10);

        let rt = Builder::new_current_thread().enable_all().build().unwrap();

        _ = std::thread::Builder::new()
            .name("system-info-worker".to_string())
            .spawn(move || {
                rt.block_on(async move {
                    let task = tokio::spawn(async move {
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
                                    let status_path =
                                        "/proc/".to_string() + pid.to_string().as_str() + "/status";

                                    let mut contents = Vec::new();

                                    if let Ok(file) =
                                        tokio::fs::File::open(status_path).await.as_mut()
                                    {
                                        let _ = file.read_to_end(&mut contents).await;

                                        let output = String::from_utf8(contents).unwrap();

                                        let mut statm_result = ProcStatus::default();

                                        for line in output.lines() {
                                            if let Some(matching) = line.strip_prefix("Name:") {
                                                statm_result.name =
                                                    matching.trim_start().to_string();
                                            } else if let Some(matching) = line.strip_prefix("Pid:")
                                            {
                                                if let Ok(parsed_pid) =
                                                    matching.trim_start().parse()
                                                {
                                                    statm_result.pid = parsed_pid;
                                                }
                                            } else if let Some(matching) =
                                                line.strip_prefix("VmSize:")
                                            {
                                                if let Some(digit_part) =
                                                    matching.trim_start().split_whitespace().next()
                                                {
                                                    if let Ok(parsed) = digit_part.parse::<u64>() {
                                                        statm_result.vm_size = parsed;
                                                    }
                                                }
                                            } else if let Some(matching) =
                                                line.strip_prefix("VmRSS:")
                                            {
                                                if let Some(digit_part) =
                                                    matching.trim_start().split_whitespace().next()
                                                {
                                                    if let Ok(parsed) = digit_part.parse::<u64>() {
                                                        statm_result.vm_rss = parsed;
                                                    }
                                                }
                                            } else if let Some(matching) =
                                                line.strip_prefix("RssShmem:")
                                            {
                                                if let Some(digit_part) =
                                                    matching.trim_start().split_whitespace().next()
                                                {
                                                    if let Ok(parsed) = digit_part.parse::<u64>() {
                                                        statm_result.rss_shem = parsed;
                                                    }
                                                }
                                                break;
                                            }
                                        }

                                        if !statm_result.name.is_empty() && statm_result.vm_size > 0
                                        {
                                            proc_stats.push(statm_result);
                                        }
                                    }
                                }
                            }
                            mem_info.process_stats = proc_stats;
                            mem_info.process_stats.sort_by_key(|u| Reverse(u.vm_size));
                            send.send(mem_info).await.unwrap();
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
}
