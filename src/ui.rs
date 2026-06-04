use crate::models::{MemCpuHistory, ProcessHistory, ProcessInfo, RingBuffer};
use crate::monitor::InfoReceiver;
use crossterm::event::{self, KeyCode};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Color;
use ratatui::symbols::{self, Marker};
use ratatui::widgets::{Axis, Chart, GraphType};
use ratatui::{
    DefaultTerminal, Frame,
    style::{Modifier, Style, Stylize},
    symbols::border,
    text::{Line, Span, Text},
    widgets::{Block, Dataset, Gauge, List, ListItem, Paragraph, Row, Table, TableState},
};
use std::collections::{BTreeMap, HashMap};
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

#[derive(Debug, Default, PartialEq)]
enum ProcessDisplayKind {
    #[default]
    List,
    Tree,
}

#[derive(Debug, Default)]
pub struct App {
    totalram: u64,
    availableram: u64,
    freeram: u64,
    cpu_usage: BTreeMap<String, f32>,
    proc_info: Vec<ProcessInfo>,
    table_order_settings: TableOrderSettings,
    watched_pid: Option<ProcessHistory>,
    cpu_history: MemCpuHistory,
    current_screen: CurrentScreen,
    process_display_kind: ProcessDisplayKind,
    exit: bool,
}

#[derive(Debug, Default)]
pub enum CurrentScreen {
    #[default]
    Main,
    Plots,
    Watch,
}

impl App {
    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        let mut info_receiver = InfoReceiver::new();

        let mut table_state = TableState::default();
        table_state.select_first();
        table_state.select_first_column();

        self.cpu_history = MemCpuHistory {
            history: RingBuffer::new(200),
        };

        while !self.exit {
            terminal.draw(|frame| self.draw(frame, &mut table_state))?;

            self.handle_events(&mut table_state);

            while let Ok(result) = info_receiver.reciver.try_recv() {
                self.freeram = result.mem_cpu_stats.free_memory;
                self.availableram = result.mem_cpu_stats.available_memory;
                self.totalram = result.mem_cpu_stats.total_memory;
                self.proc_info = result.process_stats;
                self.cpu_usage = result.mem_cpu_stats.cpu_usage.clone();

                self.sort_display_kind();

                self.cpu_history.history.push(result.mem_cpu_stats);

                if let Some(currently_observered_pid) = &mut self.watched_pid
                    && let Some(intresting_pid) = self
                        .proc_info
                        .iter()
                        .find(|x| x.pid == currently_observered_pid.pid)
                {
                    currently_observered_pid.history.push(intresting_pid.status);
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

    fn sort_display_kind(&mut self) {
        match self.process_display_kind {
            ProcessDisplayKind::List => {
                self.sort();
            }
            ProcessDisplayKind::Tree => {
                self.sort();

                let ppid_to_pid_map = self.get_ppid_to_pid_map();

                let mut correct_pid_order: Vec<u64> = Vec::with_capacity(self.proc_info.len());
                let mut pid_depths: HashMap<u64, u64> = HashMap::new();

                let root_pid = 1;
                let current_pid_depth = 0;

                correct_pid_order.push(root_pid);
                pid_depths.insert(root_pid, current_pid_depth);

                Self::add_descentants(
                    root_pid,
                    &ppid_to_pid_map,
                    &mut correct_pid_order,
                    &mut pid_depths,
                    current_pid_depth,
                );

                let pid_to_idx: HashMap<_, _> = self
                    .proc_info
                    .iter()
                    .enumerate()
                    .map(|(id, proc)| (proc.pid, id))
                    .collect();

                let mut new_proc_info: Vec<ProcessInfo> = Vec::with_capacity(self.proc_info.len());

                for pid in &correct_pid_order {
                    if let Some(&pid_idx) = pid_to_idx.get(pid)
                        && let Some(proc_info_entry) = self.proc_info.get(pid_idx)
                        && let Some(proc_pid_depth) = pid_depths.get(pid)
                    {
                        let mut clone = proc_info_entry.clone();
                        clone.pid_lvl = *proc_pid_depth;
                        new_proc_info.push(clone);
                    }
                }

                self.proc_info = new_proc_info;
            }
        }
    }

    fn add_descentants(
        pid: u64,
        hierarchy: &BTreeMap<u64, Vec<u64>>,
        order_vec: &mut Vec<u64>,
        pid_depths: &mut HashMap<u64, u64>,
        current_depth: u64,
    ) {
        let children = Self::get_children(pid, hierarchy);
        if !children.is_empty() {
            for child in children {
                order_vec.push(child);
                pid_depths.insert(child, current_depth + 1);
                Self::add_descentants(child, hierarchy, order_vec, pid_depths, current_depth + 1);
            }
        }
    }

    fn get_children(pid: u64, hierarchy: &BTreeMap<u64, Vec<u64>>) -> Vec<u64> {
        if let Some(entry) = hierarchy.get(&pid) {
            let mut descendants: Vec<u64> = Vec::with_capacity(entry.len());

            for child in entry {
                descendants.push(*child);
            }

            return descendants;
        }

        Vec::new()
    }

    fn get_ppid_to_pid_map(&self) -> BTreeMap<u64, Vec<u64>> {
        let mut ppid_to_pid_map: BTreeMap<u64, Vec<u64>> = BTreeMap::new();

        for elem in &self.proc_info {
            if let Some(ppid) = elem.parent_pid {
                ppid_to_pid_map.entry(ppid).or_default().push(elem.pid);
            }
        }
        ppid_to_pid_map
    }

    fn draw(&self, frame: &mut Frame, table_state: &mut TableState) {
        let title = Line::from(" Process Watcher ".bold());
        let instructions = Line::from(vec![
            " CPU usage plots ".into(),
            "<p>,".blue().bold(),
            " Watched process stats ".into(),
            "<l>,".blue().bold(),
            " Start watching process".into(),
            "<w>,".blue().bold(),
            " Go to main screen ".into(),
            "<m>,".blue().bold(),
            " Quit ".into(),
            "<Q> ".blue().bold(),
        ]);
        let block = Block::bordered()
            .title(title.centered())
            .title_bottom(instructions.centered())
            .border_set(border::THICK);

        let total_memory_gb = self.totalram as f32 / (1024.0 * 1024.0);
        let available_memory_gb = self.availableram as f32 / (1024.0 * 1024.0);
        let free_memory_gb = self.freeram as f32 / (1024.0 * 1024.0);

        let mem_usage = 100.0 * (1.0 - (self.availableram as f32 / self.totalram as f32));

        let counter_text = Text::from(vec![Line::from(vec![
            "Free memory (GB): ".into(),
            free_memory_gb.to_string().green(),
            ", Available memory (GB): ".into(),
            available_memory_gb.to_string().green(),
            ", Memory usage [%]: ".into(),
            mem_usage.to_string().green(),
            ", Total memory (GB): ".into(),
            total_memory_gb.to_string().yellow(),
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

        match self.current_screen {
            CurrentScreen::Main => {
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
                        format!("{:>2.4}", proc_info.status.rss_proc.to_string()),
                        format!("{:>2.4}", proc_info.status.cpu_usage.to_string()),
                        proc_info.parent_pid.map_or("<>".to_string(), |_| {
                            " |".repeat(proc_info.pid_lvl as usize).to_string()
                                + "-"
                                + proc_info.command.as_str()
                        }),
                    ]));
                }

                let items: Vec<ListItem> = self
                    .cpu_usage
                    .keys()
                    .map(|key| {
                        ListItem::new(Line::from(vec![
                            Span::raw(symbols::DOT),
                            Span::styled(
                                key,
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
            }
            CurrentScreen::Plots => {
                let col_constraints = (0..2).map(|_| Constraint::Min(9));
                let row_constraints = (0..5).map(|_| Constraint::Min(9));
                let horizontal = Layout::horizontal(col_constraints).spacing(1);
                let vertical = Layout::vertical(row_constraints).spacing(1);

                let rows = vertical.split(chunks[1]);
                let cells = rows.iter().flat_map(|&row| horizontal.split(row).to_vec());

                let mut cpu_history: BTreeMap<String, Vec<(f64, f64)>> = BTreeMap::new();

                for (id, elem) in self.cpu_history.history.buf.iter().enumerate() {
                    for elem2 in elem.cpu_usage.iter() {
                        cpu_history
                            .entry(elem2.0.clone())
                            .or_default()
                            .push((id as f64, *elem2.1 as f64));
                    }
                }

                for (cell, history) in cells.clone().zip(cpu_history) {
                    let dataset = Dataset::default()
                        .name(history.0.as_str())
                        .marker(Marker::Braille)
                        .graph_type(GraphType::Line)
                        .style(Color::Blue)
                        .data(history.1.as_slice());

                    let x_axis = Axis::default()
                        .title("Time (s)".blue())
                        .bounds([0.0, 200.0])
                        .labels(["0", "200", "400"]);

                    let y_axis = Axis::default()
                        .title(
                            format!(
                                " {} usage: ({:>2.4})%",
                                history.0,
                                history.1.first().unwrap().1.to_string().as_str()
                            )
                            .blue(),
                        )
                        .bounds([0.0, 100.0])
                        .labels(["0", "50", "100%"]);

                    let chart = Chart::new(vec![dataset]).x_axis(x_axis).y_axis(y_axis);
                    frame.render_widget(chart, cell);
                }

                if let Some(last_cell) = &cells.last() {
                    let mem_usage_history: Vec<(f64, f64)> = self
                        .cpu_history
                        .history
                        .buf
                        .iter()
                        .enumerate()
                        .map(|(x, y)| {
                            (
                                x as f64,
                                100.0 * (y.total_memory - y.available_memory) as f64
                                    / (y.total_memory as f64),
                            )
                        })
                        .collect();

                    let dataset = Dataset::default()
                        .name(format!("Used memory: ({:>2.4})%", mem_usage))
                        .marker(Marker::Braille)
                        .graph_type(GraphType::Line)
                        .style(Color::Blue)
                        .data(&mem_usage_history);

                    let x_axis = Axis::default()
                        .title("Time (s)".blue())
                        .bounds([0.0, 200.0])
                        .labels(["0", "200", "400"]);

                    let y_axis = Axis::default()
                        .title(format!("Used memory: ({:>2.4})%", mem_usage).blue())
                        .bounds([0.0, 100.0])
                        .labels(["0", "50", "100%"]);

                    let chart = Chart::new(vec![dataset]).x_axis(x_axis).y_axis(y_axis);
                    frame.render_widget(chart, *last_cell);
                }
            }
            CurrentScreen::Watch => {
                if let Some(observed_pid) = &self.watched_pid {
                    let data: Vec<(u64, u64, u64, f32, f32)> = observed_pid
                        .history
                        .buf
                        .iter()
                        .map(|x| (x.vm_size, x.vm_rss, x.rss_shem, x.rss_proc, x.cpu_usage))
                        .collect();

                    let header =
                        Row::new(["VM_SIZE", "VM_RSS", "RSS_SHEM", "RSS_PROC", "CPU USAGE"])
                            .style(Style::new().bold())
                            .bottom_margin(1);

                    let mut rows: Vec<Row> = Vec::new();

                    for entry in &data {
                        rows.push(Row::new([
                            entry.0.to_string(),
                            entry.1.to_string(),
                            entry.2.to_string(),
                            entry.3.to_string(),
                            entry.4.to_string(),
                        ]));
                    }

                    let widths = [
                        Constraint::Max(12),
                        Constraint::Max(12),
                        Constraint::Max(12),
                        Constraint::Max(10),
                        Constraint::Max(10),
                    ];

                    let table = Table::new(rows, widths)
                        .header(header)
                        .column_spacing(1)
                        .style(Color::White)
                        .row_highlight_style(Style::new().on_black().bold())
                        .column_highlight_style(Color::Gray)
                        .cell_highlight_style(Style::new().reversed().yellow())
                        .highlight_symbol("() ");

                    frame.render_widget(table, chunks[1]);
                }
            }
        }
    }

    fn on_sort_requested_event(&mut self, table_state: &mut TableState) {
        if let Some(idx) = table_state.selected_column()
            && let Some(col) = UI_COLUMNS.get(idx)
        {
            if self.table_order_settings.order_by_field != *col {
                self.table_order_settings.order_by_field = *col;
                self.table_order_settings.order = SortOrder::Descending;
            } else if self.table_order_settings.order == SortOrder::Ascending {
                self.table_order_settings.order = SortOrder::Descending;
            } else {
                self.table_order_settings.order = SortOrder::Ascending;
            }
            self.sort_display_kind();
            return;
        }
        self.sort_display_kind();
    }

    fn on_watch_pid_event(&mut self, table_state: &mut TableState) {
        if let Some(idx) = table_state.selected_cell()
            && let Some(selected_process) = self.proc_info.get(idx.0)
        {
            if let Some(currently_observered_pid) = &self.watched_pid
                && currently_observered_pid.pid == selected_process.pid
            {
                return;
            }

            self.watched_pid = Some(ProcessHistory {
                pid: selected_process.pid,
                history: RingBuffer::new(200),
            });
        }
    }

    fn on_toogle_pid_view_event(&mut self) {
        match self.process_display_kind {
            ProcessDisplayKind::List => self.process_display_kind = ProcessDisplayKind::Tree,
            ProcessDisplayKind::Tree => self.process_display_kind = ProcessDisplayKind::List,
        }

        self.sort_display_kind();
    }

    fn handle_events(&mut self, table_state: &mut TableState) {
        if let Ok(true) = event::poll(Duration::from_millis(100))
            && let Ok(event::Event::Key(key)) = event::read()
        {
            match key.code {
                KeyCode::Char('q') => self.exit(),
                KeyCode::Down => table_state.select_next(),
                KeyCode::Up => table_state.select_previous(),
                KeyCode::Right => table_state.select_next_column(),
                KeyCode::Left => table_state.select_previous_column(),
                KeyCode::Char('l') => {
                    self.current_screen = CurrentScreen::Watch;
                }
                KeyCode::Char('m') => {
                    self.current_screen = CurrentScreen::Main;
                }
                KeyCode::Char('p') => {
                    self.current_screen = CurrentScreen::Plots;
                }
                KeyCode::Char('s') => {
                    self.on_sort_requested_event(table_state);
                }
                KeyCode::Char('w') => {
                    self.on_watch_pid_event(table_state);
                }
                KeyCode::Char('t') => {
                    self.on_toogle_pid_view_event();
                }
                _ => {}
            }
        }
    }

    fn exit(&mut self) {
        self.exit = true;
    }
}
