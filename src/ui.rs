use crate::models::{ProcessHistory, ProcessInfo, ProcessStatus, RingBuffer};
use crate::monitor::InfoReceiver;
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
use std::collections::BTreeMap;
use std::time::Duration;
const UI_COLUMNS: [ProcessUiColumn; 7] = [
    ProcessUiColumn::Pid,
    ProcessUiColumn::VmSize,
    ProcessUiColumn::VmRss,
    ProcessUiColumn::RssShem,
    ProcessUiColumn::RssProc,
    ProcessUiColumn::CpuUsage,
    ProcessUiColumn::Command,
];

#[derive(Debug, Default, PartialEq, Clone, Copy)]
enum ProcessUiColumn {
    Command,
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
    totalram: u64,
    freeram: u64,
    cpu_usage: BTreeMap<String, f32>,
    proc_info: Vec<ProcessInfo>,
    table_order_settings: TableOrderSettings,
    watched_pid: Option<ProcessHistory>,
    exit: bool,
}

impl App {
    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
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

                self.sort();

                if let Some(currently_observered_pid) = &mut self.watched_pid {
                    if let Some(intresting_pid) = self
                        .proc_info
                        .iter()
                        .find(|x| x.pid == currently_observered_pid.pid)
                    {
                        currently_observered_pid
                            .history
                            .push(intresting_pid.status.clone());
                    }
                }
            }
        }
        Ok(())
    }

    fn sort(&mut self) {
        match self.table_order_settings.order_by_field {
            ProcessUiColumn::Command => self.proc_info.sort_by(|a, b| a.command.cmp(&b.command)),
            ProcessUiColumn::Pid => self.proc_info.sort_by_key(|u| u.pid),
            ProcessUiColumn::VmSize => self.proc_info.sort_by_key(|u| u.status.vm_size),
            ProcessUiColumn::VmRss => self.proc_info.sort_by_key(|u| u.status.vm_rss),
            ProcessUiColumn::RssShem => self.proc_info.sort_by_key(|u| u.status.rss_shem),
            ProcessUiColumn::RssProc => self
                .proc_info
                .sort_by(|u, v| u.status.rss_proc.total_cmp(&v.status.rss_proc)),
            ProcessUiColumn::CpuUsage => self
                .proc_info
                .sort_by(|u, v| u.status.cpu_usage.total_cmp(&v.status.cpu_usage)),
        }

        if self.table_order_settings.order == SortOrder::Descending {
            self.proc_info.reverse();
        }
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
                Constraint::Max(1),
            ])
            .spacing(1)
            .split(frame.area());

        let paragraph = Paragraph::new(counter_text).centered().block(block);

        frame.render_widget(paragraph, chunks[0]);

        let header = Row::new(["PID", "VIRT", "RSS", "SHR", "MEM(%)", "CPU(%)", "cmd"])
            .style(Style::new().bold())
            .bottom_margin(1);

        let mut rows: Vec<Row> = Vec::new();

        for proc_info in &self.proc_info {
            rows.push(Row::new([
                proc_info.pid.to_string(),
                proc_info.status.vm_size.to_string(),
                proc_info.status.vm_rss.to_string(),
                proc_info.status.rss_shem.to_string(),
                proc_info.status.rss_proc.to_string(),
                format!("{:>2.4}", proc_info.status.cpu_usage.to_string()),
                proc_info.command.clone(),
            ]));
        }

        let items: Vec<ListItem> = self
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

        for (i, (_, usage)) in self.cpu_usage.iter().take(max_rows).enumerate() {
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
            Constraint::Max(6),
            Constraint::Max(12),
            Constraint::Max(12),
            Constraint::Max(10),
            Constraint::Max(10),
            Constraint::Max(10),
            Constraint::Min(30),
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

        if let Some(observed_pid) = &self.watched_pid {
            if !observed_pid.history.buf.is_empty() {
                let text = Text::from(
                    observed_pid.pid.to_string()
                        + ", Samples:"
                        + observed_pid.history.buf.iter().len().to_string().as_str(),
                );

                let paragraph_2 = Paragraph::new(text).centered();
                frame.render_widget(paragraph_2, chunks[3]);
            }
        }
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
                                    self.sort();
                                    return;
                                }
                            }

                            self.sort();
                        }
                        KeyCode::Char('w') => {
                            if let Some(idx) = table_state.selected_cell() {
                                if let Some(selected_process) = self.proc_info.get(idx.0) {
                                    if let Some(currently_observered_pid) = &self.watched_pid {
                                        if currently_observered_pid.pid == selected_process.pid {
                                            return;
                                        }
                                    }

                                    self.watched_pid = Some(ProcessHistory {
                                        pid: selected_process.pid,
                                        history: RingBuffer::new(200),
                                    });
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
