mod config;
mod context;
mod ai;
mod pty;

use std::io;

fn main() -> io::Result<()> {
    let config = config::Config::load();
    let db = context::Db::open();
    pty::run(&config, &db)
}
