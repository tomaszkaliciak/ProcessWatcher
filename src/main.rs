mod models;
mod monitor;
mod parser;
mod ui;

use std::io;
use ui::App;

fn main() -> io::Result<()> {
    ratatui::run(|terminal| App::default().run(terminal))
}
