use libc;

use std::fs;
use std::io;
use std::mem;
use std::num;
use std::ops::Add;

use crossterm::event::{self, KeyCode};
use ratatui::style::Color;
use ratatui::{
    DefaultTerminal, Frame,
    buffer::Buffer,
    layout::Rect,
    style::{Style, Stylize},
    symbols::border,
    text::{Line, Span, Text},
    widgets::{Block, List, ListItem, Paragraph, Widget},
};
use tokio::fs::File;
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

        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;

            self.handle_events();

            while let Ok(result) = info_receiver.reciver.try_recv() {
                self.freeram = result.free_memory;
                self.totalram = result.total_memory;
                self.proc_info = result.process_stats;
            }
        }
        Ok(())
    }

    fn draw(&self, frame: &mut Frame) {
        frame.render_widget(self, frame.area());
    }

    fn handle_events(&mut self) {
        match event::poll(Duration::from_millis(100)) {
            Ok(true) => {
                if let Ok(event::Event::Key(key)) = event::read() {
                    match key.code {
                        KeyCode::Char('q') => self.exit(),
                        KeyCode::Char('u') => self.update(),
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

impl Widget for &App {
    fn render(self, area: Rect, buf: &mut Buffer) {
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
            "Totalram: ".into(),
            self.totalram.to_string().yellow(),
            " Freeram: ".into(),
            self.freeram.to_string().green(),
        ])]);

        Paragraph::new(counter_text)
            .centered()
            .block(block)
            .render(area, buf);

        let mut list_items = Vec::<ListItem>::new();

        for proc_info in &self.proc_info {
            list_items.push(ListItem::new(Line::from(Span::styled(
                format!(
                    "{} ; PID: {} ;VM {} KB; RSS: {} KB",
                    proc_info.name, proc_info.pid, proc_info.vm_size, proc_info.vm_size
                ),
                Style::default().fg(Color::Yellow),
            ))));
        }
        List::new(list_items).render(area, buf);
    }
}

impl InfoReceiver {
    pub fn new() -> InfoReceiver {
        let (send, recv) = mpsc::channel(10);

        let rt = Builder::new_current_thread().enable_all().build().unwrap();

        std::thread::spawn(move || {
            rt.block_on(async move {
                let task = tokio::spawn(async move {
                    let mut interval = time::interval(Duration::from_secs(10));
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

                                if let Ok(file) = tokio::fs::File::open(status_path).await.as_mut()
                                {
                                    let _ = file.read_to_end(&mut contents).await;

                                    let output = String::from_utf8(contents).unwrap();

                                    let mut statm_result = ProcStatus::default();

                                    for line in output.lines() {
                                        if line.starts_with("Name:") {
                                            if let Some(result) = line.split(":").nth(1) {
                                                statm_result.name = String::from(result);
                                            }
                                        } else if line.starts_with("Pid:") {
                                            let pid: u64 = line
                                                .chars()
                                                .filter(|c| c.is_ascii_digit())
                                                .collect::<String>()
                                                .parse()
                                                .unwrap_or(0);
                                            statm_result.pid = pid;
                                        } else if line.starts_with("VmSize:") {
                                            let number: u64 = line
                                                .chars()
                                                .filter(|c| c.is_ascii_digit())
                                                .collect::<String>()
                                                .parse()
                                                .unwrap_or(0);
                                            statm_result.vm_size = number;
                                        } else if line.starts_with("VmRSS:") {
                                            let number: u64 = line
                                                .chars()
                                                .filter(|c| c.is_ascii_digit())
                                                .collect::<String>()
                                                .parse()
                                                .unwrap_or(0);
                                            statm_result.vm_rss = number;
                                        }
                                    }

                                    if !statm_result.name.is_empty() {
                                        proc_stats.push(statm_result);
                                    }
                                }
                            }
                        }
                        mem_info.process_stats = proc_stats;
                        mem_info.process_stats.sort_by_key(|u| u.vm_size);
                        mem_info.process_stats.reverse();
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
}
