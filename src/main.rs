use libc;

use std::io;
use std::mem;

use tokio::runtime::Builder;
use tokio::sync::mpsc;
use tokio::time::{self, Duration};

use crossterm::event::{self, KeyCode};
use ratatui::{
    DefaultTerminal, Frame,
    buffer::Buffer,
    layout::Rect,
    style::Stylize,
    symbols::border,
    text::{Line, Text},
    widgets::{Block, Paragraph, Widget},
};

pub struct MemInfo {
    total_memory: u64,
    free_memory: u64,
}

impl MemInfo {
    pub fn send(state: (u64, u64)) -> MemInfo {
        MemInfo {
            total_memory: state.0,
            free_memory: state.1,
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
    }
}

impl InfoReceiver {
    pub fn new() -> InfoReceiver {
        let (send, recv) = mpsc::channel(10);

        let rt = Builder::new_current_thread().enable_all().build().unwrap();

        std::thread::spawn(move || {
            rt.block_on(async move {
                let task = tokio::spawn(async move {
                    let mut interval = time::interval(Duration::from_secs(2));
                    loop {
                        interval.tick().await;

                        unsafe {
                            let mut info: libc::sysinfo = mem::zeroed();

                            if libc::sysinfo(&mut info) == 0 {
                                let mem_info: MemInfo = MemInfo {
                                    total_memory: (info.totalram),
                                    free_memory: (info.freeram),
                                };
                                send.send(mem_info).await.unwrap();
                            } else {
                                eprintln!("Failed to get system info.");
                            }
                        }
                    }
                });
                task.await.unwrap();
            });
        });

        InfoReceiver { reciver: recv }
    }
}
