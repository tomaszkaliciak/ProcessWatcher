mod models;
mod monitor;
mod parser;
mod ui;

use cli_log::*;
use std::io;
use ui::App;

fn main() -> io::Result<()> {
    init_cli_log!("ProcessWatcher");

    ratatui::run(|terminal| App::default().run(terminal))
}
