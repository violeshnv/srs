use std::{io, path::PathBuf};

use crate::data::DEFAULT_DATA_PATH;
use crate::tui::app::App;

fn main() -> io::Result<()> {
    env_logger::init();

    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_DATA_PATH));

    let mut app = App::new(path);
    ratatui::run(|terminal| app.run(terminal))
}

mod data;
mod tui;
