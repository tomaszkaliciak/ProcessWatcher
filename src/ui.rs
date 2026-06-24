use crate::models::{MemCpuHistory, ProcessHistory, ProcessInfo, RingBuffer};
use crate::monitor::InfoReceiver;
use cli_log::info;
use crossterm::event::{
    self, EnableMouseCapture, Event, KeyCode, MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Color;
use ratatui::symbols::{self, Marker};
use ratatui::widgets::{Axis, Borders, Chart, Clear, GraphType, Wrap};
use ratatui::{
    DefaultTerminal, Frame,
    style::{Modifier, Style, Stylize},
    symbols::border,
    text::{Line, Span, Text},
    widgets::{Block, Dataset, Gauge, List, ListItem, Paragraph, Row, Table, TableState},
};
use std::collections::{BTreeMap, HashMap};
use std::fs::{OpenOptions, metadata};
use std::io::{self, BufWriter, Write};
use std::time::Duration;

use tui_input::Input;
use tui_input::backend::crossterm::EventHandler;

const UI_COLUMNS: [ProcessUiColumn; 8] = [
    ProcessUiColumn::Pid,
    ProcessUiColumn::VmSize,
    ProcessUiColumn::VmRss,
    ProcessUiColumn::RssShem,
    ProcessUiColumn::RssProc,
    ProcessUiColumn::CpuUsage,
    ProcessUiColumn::Uptime,
    ProcessUiColumn::Command,
];

#[derive(Debug, Default, PartialEq, Clone, Copy)]
enum ProcessUiColumn {
    Pid,
    VmSize,
    #[default]
    VmRss,
    RssShem,
    RssProc,
    CpuUsage,
    Uptime,
    Command,
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

#[derive(Debug, Default, PartialEq)]
enum InputMode {
    #[default]
    Normal,
    EditingFilter,
    EditingSearch,
}

#[derive(Debug, Default)]
struct SearchState {
    current_input: Input,
    current_match_idx: Option<usize>,
    matched_rows: Vec<usize>,
}

#[derive(Debug, Default)]
pub struct App {
    totalram: u64,
    uptime: u64,
    availableram: u64,
    freeram: u64,
    cpu_usage: BTreeMap<String, f32>,
    proc_info: Vec<ProcessInfo>,
    table_order_settings: TableOrderSettings,
    watched_pid_history: Option<ProcessHistory>,
    watched_pid: u64,
    cpu_history: MemCpuHistory,
    current_screen: CurrentScreen,
    process_display_kind: ProcessDisplayKind,
    current_input_mode: InputMode,
    current_filter_input: Input,
    search_state: SearchState,
    table_area: Rect,
    column_areas: Vec<Rect>,
    table_state: TableState,
    save_watched_pid_filename_input: Input,
    exit: bool,
}
#[derive(Debug, Default, PartialEq)]
pub enum CurrentScreen {
    #[default]
    Main,
    Plots,
    Watch,
    WatchSaveFileProvidePath,
    WatchSaveFileConfirm,
}

impl App {
    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        let mut info_receiver = InfoReceiver::new();

        self.table_state.select_first();
        self.table_state.select_first_column();

        self.cpu_history = MemCpuHistory {
            history: RingBuffer::new(200),
        };

        execute!(io::stdout(), EnableMouseCapture)?;

        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;

            self.handle_events();

            while let Ok(result) = info_receiver.reciver.try_recv() {
                self.uptime = result.uptime;
                self.freeram = result.mem_cpu_stats.free_memory;
                self.availableram = result.mem_cpu_stats.available_memory;
                self.totalram = result.mem_cpu_stats.total_memory;
                self.proc_info = result.process_stats;
                self.cpu_usage = result.mem_cpu_stats.cpu_usage.clone();

                self.sort_display_kind();

                self.cpu_history.history.push(result.mem_cpu_stats);

                if let Some(currently_observered_pid) = &mut self.watched_pid_history
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

    fn format_to_hh_mm_ss(total_seconds: u64) -> String {
        let hours = total_seconds / 3600;
        let minutes = (total_seconds % 3600) / 60;
        let seconds = total_seconds % 60;

        format!("{:01}:{:02}:{:02}", hours, minutes, seconds)
    }

    fn sort(&mut self) {
        match self.table_order_settings.order_by_field {
            ProcessUiColumn::Command => self.proc_info.sort_by(|a, b| a.command.cmp(&b.command)),
            ProcessUiColumn::Pid => self.proc_info.sort_by_key(|u| u.pid),
            ProcessUiColumn::VmSize => self.proc_info.sort_by_key(|u| u.status.vm_size),
            ProcessUiColumn::VmRss => self.proc_info.sort_by_key(|u| u.status.vm_rss),
            ProcessUiColumn::RssShem => self.proc_info.sort_by_key(|u| u.status.rss_shem),
            ProcessUiColumn::Uptime => self.proc_info.sort_by_key(|u| u.status.uptime),
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

        if self.search_state.is_active() {
            self.search_state.calculate_matching_rows(&self.proc_info);
        }
    }

    fn add_descentants(
        pid: u64,
        hierarchy: &BTreeMap<u64, Vec<u64>>,
        order_vec: &mut Vec<u64>,
        pid_depths: &mut HashMap<u64, u64>,
        current_depth: u64,
    ) {
        let children: &[u64] = hierarchy.get(&pid).map_or(&[], |x| x.as_slice());
        if !children.is_empty() {
            for child in children {
                order_vec.push(*child);
                pid_depths.insert(*child, current_depth + 1);
                Self::add_descentants(*child, hierarchy, order_vec, pid_depths, current_depth + 1);
            }
        }
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

    fn draw(&mut self, frame: &mut Frame) {
        let title = Line::from(" Process Watcher ".bold());

        let mut intructions: Vec<Span> = Vec::new();

        match self.current_screen {
            CurrentScreen::Main => match self.current_input_mode {
                InputMode::Normal => {
                    intructions.append(&mut vec![
                        " CPU usage plots ".into(),
                        "<p>,".blue().bold(),
                        " Watched process stats ".into(),
                        "<l>,".blue().bold(),
                        (if self.current_filter_input.to_string().is_empty() {
                            " Filter ".into()
                        } else {
                            " FILTER ".into()
                        }),
                        "<f>,".blue().bold(),
                        " Search ".into(),
                        "<s>,".blue().bold(),
                        " Toggle tree/list view ".into(),
                        "<t>,".blue().bold(),
                        " Start watching process".into(),
                        "<w>,".blue().bold(),
                        " Sort".into(),
                        "<r>,".blue().bold(),
                    ]);
                }
                InputMode::EditingFilter => {
                    intructions.append(&mut vec![
                        " Done ".into(),
                        "<ENTER>,".blue().bold(),
                        " Cancel ".into(),
                        "<ESC>,".blue().bold(),
                    ]);
                }
                InputMode::EditingSearch => {
                    intructions.append(&mut vec![
                        " Next ".into(),
                        "<KEY_DOWN>,".blue().bold(),
                        " Previous ".into(),
                        "<KEY_UP>,".blue().bold(),
                        " Cancel ".into(),
                        "<ESC>,".blue().bold(),
                    ]);
                }
            },
            CurrentScreen::Plots => {
                intructions.append(&mut vec![
                    " Main view ".into(),
                    "<m>,".blue().bold(),
                    " Watched process stats ".into(),
                    "<l>,".blue().bold(),
                ]);
            }
            CurrentScreen::Watch => {
                intructions.append(&mut vec![
                    " Main view ".into(),
                    "<m>,".blue().bold(),
                    " CPU usage plots ".into(),
                    "<p>,".blue().bold(),
                    " Save data ".into(),
                    "<s>,".blue().bold(),
                ]);
            }
            CurrentScreen::WatchSaveFileProvidePath => {}
            CurrentScreen::WatchSaveFileConfirm => {
                intructions.append(&mut vec![
                    " Confirm ".into(),
                    "<Y>,".blue().bold(),
                    " Cancel ".into(),
                    "<N>,".blue().bold(),
                ]);
            }
        }

        if self.current_screen != CurrentScreen::WatchSaveFileProvidePath
            && self.current_screen != CurrentScreen::WatchSaveFileConfirm
        {
            intructions.append(&mut vec![" Quit ".into(), "<Q> ".blue().bold()]);
        }

        let instructions = Line::from(intructions);

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
            format!("{:.3}", free_memory_gb).green(),
            ", Available memory (GB): ".into(),
            format!("{:.3}", available_memory_gb).green(),
            ", Memory usage [%]: ".into(),
            format!("{:.2}", mem_usage).green(),
            ", Total memory (GB): ".into(),
            format!("{:.2}", total_memory_gb).green(),
            ", Uptime(s): ".into(),
            Self::format_to_hh_mm_ss(self.uptime).yellow(),
        ])]);

        let chunks = if self.current_input_mode == InputMode::Normal {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(1),
                    Constraint::Length(8),
                    Constraint::Max(1),
                ])
                .spacing(1)
                .split(frame.area())
        } else {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(1),
                    Constraint::Length(8),
                    Constraint::Max(1),
                    Constraint::Max(3),
                ])
                .spacing(1)
                .split(frame.area())
        };

        if self.current_input_mode == InputMode::EditingFilter {
            self.render_text_input(frame, &self.current_filter_input, "Filter: ", chunks[4]);
        } else if self.current_input_mode == InputMode::EditingSearch {
            self.render_text_input(
                frame,
                &self.search_state.current_input,
                "Search: ",
                chunks[4],
            );
        }

        let paragraph = Paragraph::new(counter_text).centered().block(block);

        frame.render_widget(paragraph, chunks[0]);

        match self.current_screen {
            CurrentScreen::Main => {
                let header = Row::new([
                    "PID", "VIRT", "RSS", "SHR", "MEM(%)", "CPU(%)", "Uptime", "cmd",
                ])
                .style(Style::new().bold())
                .bottom_margin(1);

                let mut rows: Vec<Row> = Vec::new();

                let is_filter_mode_active = !self.current_filter_input.to_string().is_empty();

                for (idx, proc_info) in self.proc_info.iter().enumerate() {
                    if is_filter_mode_active
                        && !proc_info
                            .command
                            .contains(self.current_filter_input.to_string().as_str())
                    {
                        continue;
                    }

                    if self.search_state.is_active() && self.search_state.get_current() == Some(idx)
                    {
                        self.table_state.select(Some(idx));
                    }

                    let pid_row = if self.watched_pid == proc_info.pid {
                        "<w>".to_string() + proc_info.pid.to_string().as_str()
                    } else {
                        proc_info.pid.to_string()
                    };

                    rows.push(Row::new([
                        pid_row,
                        proc_info.status.vm_size.to_string(),
                        proc_info.status.vm_rss.to_string(),
                        proc_info.status.rss_shem.to_string(),
                        format!("{:>2.4}", proc_info.status.rss_proc.to_string()),
                        format!("{:>2.4}", proc_info.status.cpu_usage.to_string()),
                        Self::format_to_hh_mm_ss(proc_info.status.uptime),
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
                    Constraint::Max(10),
                    Constraint::Max(12),
                    Constraint::Max(12),
                    Constraint::Max(10),
                    Constraint::Max(6),
                    Constraint::Max(6),
                    Constraint::Max(8),
                    Constraint::Min(30),
                ];

                let column_layouts = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints(widths)
                    .split(chunks[1]);

                self.table_area = chunks[1];
                self.column_areas = column_layouts.to_vec();

                let table = Table::new(rows, widths)
                    .header(header)
                    .column_spacing(1)
                    .style(Color::White)
                    .row_highlight_style(Style::new().on_black().bold())
                    .column_highlight_style(Color::Gray)
                    .cell_highlight_style(Style::new().reversed().yellow())
                    .highlight_symbol("() ");

                frame.render_stateful_widget(table, chunks[1], &mut self.table_state);
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
                if let Some(observed_pid) = &self.watched_pid_history {
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
            CurrentScreen::WatchSaveFileProvidePath => {
                let centered_area = frame
                    .area()
                    .centered(Constraint::Percentage(40), Constraint::Percentage(5));
                frame.render_widget(Clear, centered_area);

                self.render_text_input(
                    frame,
                    &self.save_watched_pid_filename_input,
                    "Enter filename. Press OK to confirm, ESC to cancel",
                    centered_area,
                );
            }
            CurrentScreen::WatchSaveFileConfirm => {
                let popup_block = Block::default()
                    .title("Y/N")
                    .borders(Borders::NONE)
                    .style(Style::default().bg(Color::DarkGray));

                let exit_text = Text::styled(
                    format!(
                        "Are you sure that you want to save output to :`{}`? (Y/N)",
                        self.save_watched_pid_filename_input,
                    ),
                    Style::default().fg(Color::Red),
                );

                let exit_paragraph = Paragraph::new(exit_text)
                    .block(popup_block)
                    .wrap(Wrap { trim: false });

                let area = centered_rect(60, 25, frame.area());
                frame.render_widget(exit_paragraph, area);
            }
        }
    }

    fn on_sort_requested_event(&mut self) {
        if let Some(idx) = self.table_state.selected_column()
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

    fn on_watch_pid_event(&mut self) {
        if let Some(idx) = self.table_state.selected_cell()
            && let Some(selected_process) = self.proc_info.get(idx.0)
        {
            if let Some(currently_observered_pid) = &self.watched_pid_history
                && currently_observered_pid.pid == selected_process.pid
            {
                return;
            }

            self.watched_pid = selected_process.pid;
            self.watched_pid_history = Some(ProcessHistory {
                pid: selected_process.pid,
                history: RingBuffer::new(200),
            });
        }
    }

    fn render_text_input(
        &self,
        frame: &mut Frame,
        input_field: &Input,
        block_title: &str,
        area: Rect,
    ) {
        let width = area.width.max(3) - 3;
        let scroll = input_field.visual_scroll(width as usize);
        let style = Color::Yellow;

        let input = Paragraph::new(input_field.value())
            .style(style)
            .scroll((0, scroll as u16))
            .block(Block::bordered().title(block_title));
        frame.render_widget(input, area);

        let x = input_field.visual_cursor().max(scroll) - scroll + 1;
        frame.set_cursor_position((area.x + x as u16, area.y + 1))
    }

    fn on_toogle_pid_view_event(&mut self) {
        match self.process_display_kind {
            ProcessDisplayKind::List => self.process_display_kind = ProcessDisplayKind::Tree,
            ProcessDisplayKind::Tree => self.process_display_kind = ProcessDisplayKind::List,
        }

        self.sort_display_kind();
    }

    fn on_mouse_event(&mut self, event: MouseEvent) {
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }

        let mouse_x = event.column;
        let mouse_y = event.row;

        if mouse_y >= self.table_area.top() && mouse_y < self.table_area.bottom() {
            let mut clicked_column_idx = None;
            for (idx, col_rect) in self.column_areas.iter().enumerate() {
                if mouse_x >= col_rect.left() && mouse_x < col_rect.right() {
                    clicked_column_idx = Some(idx);
                    break;
                }
            }

            if let Some(col_idx) = clicked_column_idx {
                let header_height = 2;

                if mouse_y < self.table_area.top() + header_height
                    && let Some(col) = UI_COLUMNS.get(col_idx)
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
                }
            }
        }
    }

    fn is_csv_path_correct(path: &str) -> bool {
        if let Ok(path_info) = metadata(path) {
            if path_info.is_file() {
                return true;
            } else if path_info.is_dir() {
                return false;
            }
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path);

        file.is_ok()
    }

    fn save(&mut self) {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.save_watched_pid_filename_input.to_string());

        if file.is_err() || self.watched_pid_history.is_none() {
            return;
        }

        let mut writer = BufWriter::new(file.unwrap());

        let _ = writeln!(writer, "VM_SIZE, VM_RSS, RSS_SHEM, RSS_PROC, CPU USAGE");

        for entry in self
            .watched_pid_history
            .as_ref()
            .unwrap()
            .history
            .buf
            .iter()
        {
            let _ = writer.write_all(
                format!(
                    "{}, {}, {},{}, {}\n",
                    entry.vm_size, entry.vm_rss, entry.rss_shem, entry.rss_proc, entry.cpu_usage
                )
                .as_bytes(),
            );
        }

        writer.flush().unwrap();
    }

    fn on_key_event(&mut self, event: Event) {
        if let Event::Key(key) = event {
            if self.current_screen == CurrentScreen::WatchSaveFileConfirm {
                if key.code == KeyCode::Char('y') || key.code == KeyCode::Char('Y') {
                    self.save();
                    self.save_watched_pid_filename_input.reset();
                    self.current_screen = CurrentScreen::Watch;
                } else if key.code == KeyCode::Char('n') || key.code == KeyCode::Char('N') {
                    self.current_screen = CurrentScreen::WatchSaveFileProvidePath;
                }
            } else if self.current_screen == CurrentScreen::WatchSaveFileProvidePath {
                if key.code == KeyCode::Esc {
                    self.save_watched_pid_filename_input.reset();
                    self.current_screen = CurrentScreen::Watch;
                } else if key.code == KeyCode::Enter
                    && !self.save_watched_pid_filename_input.to_string().is_empty()
                    && Self::is_csv_path_correct(self.save_watched_pid_filename_input.value())
                {
                    self.current_screen = CurrentScreen::WatchSaveFileConfirm;
                } else {
                    self.save_watched_pid_filename_input.handle_event(&event);
                }
            } else if self.current_input_mode == InputMode::EditingFilter {
                if key.code == KeyCode::Esc {
                    self.current_filter_input.reset();
                    self.current_input_mode = InputMode::Normal;
                } else if key.code == KeyCode::Enter {
                    self.current_input_mode = InputMode::Normal;
                } else {
                    self.current_filter_input.handle_event(&event);
                }
            } else if self.current_input_mode == InputMode::EditingSearch {
                if key.code == KeyCode::Esc {
                    self.search_state.clear();
                    self.current_input_mode = InputMode::Normal;
                } else if key.code == KeyCode::Down {
                    self.search_state.seek_next();
                } else if key.code == KeyCode::Up {
                    self.search_state.seek_previous();
                } else {
                    self.search_state.current_input.handle_event(&event);
                    self.search_state.calculate_matching_rows(&self.proc_info);
                }
            } else {
                match key.code {
                    KeyCode::Char('q') => self.exit(),
                    KeyCode::Down => {
                        if let CurrentScreen::Main = self.current_screen {
                            self.table_state.select_next();
                        };
                    }
                    KeyCode::Up => {
                        if let CurrentScreen::Main = self.current_screen {
                            self.table_state.select_previous();
                        }
                    }
                    KeyCode::Right => {
                        if let CurrentScreen::Main = self.current_screen {
                            self.table_state.select_next_column();
                        }
                    }
                    KeyCode::Left => {
                        if let CurrentScreen::Main = self.current_screen {
                            self.table_state.select_previous_column();
                        }
                    }
                    KeyCode::Char('f') => {
                        if let CurrentScreen::Main = self.current_screen {
                            self.current_input_mode = InputMode::EditingFilter;
                        };
                    }
                    KeyCode::Char('s') => {
                        if let CurrentScreen::Main = self.current_screen {
                            self.current_input_mode = InputMode::EditingSearch;
                        } else if let CurrentScreen::Watch = self.current_screen {
                            self.current_screen = CurrentScreen::WatchSaveFileProvidePath;
                        };
                    }
                    KeyCode::Esc => {
                        if let CurrentScreen::Main = self.current_screen {
                            self.current_input_mode = InputMode::Normal;
                        };
                    }
                    KeyCode::Char('l') => {
                        self.current_screen = CurrentScreen::Watch;
                    }
                    KeyCode::Char('m') => {
                        self.current_screen = CurrentScreen::Main;
                    }
                    KeyCode::Char('p') => {
                        self.current_screen = CurrentScreen::Plots;
                    }
                    KeyCode::Char('r') => {
                        if let CurrentScreen::Main = self.current_screen {
                            self.on_sort_requested_event();
                        }
                    }
                    KeyCode::Char('w') => {
                        if let CurrentScreen::Main = self.current_screen {
                            self.on_watch_pid_event();
                        }
                    }
                    KeyCode::Char('t') => {
                        if let CurrentScreen::Main = self.current_screen {
                            self.on_toogle_pid_view_event();
                        };
                    }
                    _ => {}
                }
            }
        }
    }

    fn handle_events(&mut self) {
        if let Ok(true) = event::poll(Duration::from_millis(100))
            && let Ok(event) = event::read()
        {
            match event {
                event::Event::Mouse(mouse_event) => self.on_mouse_event(mouse_event),
                event::Event::Key(_) => self.on_key_event(event),
                _ => {}
            }
        }
    }

    fn exit(&mut self) {
        self.exit = true;
    }
}

impl SearchState {
    pub fn is_active(&self) -> bool {
        !self.current_input.to_string().is_empty()
    }

    pub fn clear(&mut self) {
        self.current_match_idx = None;
        self.matched_rows.clear();
        self.current_input.reset();
    }

    pub fn calculate_matching_rows(&mut self, proc_info: &[ProcessInfo]) {
        self.matched_rows = proc_info
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry
                    .command
                    .contains(self.current_input.to_string().as_str())
            })
            .map(|(idx, _)| idx)
            .collect();
    }

    pub fn get_current(&mut self) -> Option<usize> {
        if let None = self.current_match_idx
            && !self.matched_rows.is_empty()
        {
            self.current_match_idx = Some(0);
        }

        if let Some(current_idx) = self.current_match_idx {
            return self.matched_rows.get(current_idx).cloned();
        }
        None
    }

    pub fn seek_next(&mut self) {
        if let Some(currently_matched) = self.current_match_idx
            && currently_matched + 1 < self.matched_rows.len()
        {
            self.current_match_idx = Some(currently_matched + 1);
        }
    }

    pub fn seek_previous(&mut self) {
        if let Some(currently_matched) = self.current_match_idx
            && currently_matched > 0
        {
            self.current_match_idx = Some(currently_matched - 1);
        }
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
