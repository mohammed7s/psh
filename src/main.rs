mod config;
mod context;
mod ai;
mod pty;

use std::io;

fn main() -> io::Result<()> {
    let config = config::Config::load();
    context::refresh_machine_context();
    let db = context::History::open();
    pty::run(&config, &db)
}
