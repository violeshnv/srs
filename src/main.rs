use std::io;

use crate::tui::home::Home;

fn main() -> io::Result<()> {
    env_logger::init();
    ratatui::run(|terminal| Home::default().run(terminal))
}

mod data;
mod tui;
