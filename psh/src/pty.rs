use crate::config::Config;
use crate::context::{Db, Entry};
use crate::ai;

use crossterm::terminal::{enable_raw_mode, disable_raw_mode};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;

// ── State machine ────────────────────────────────────────────────────────────

enum Mode {
    // Waiting for first character of a new line
    Idle,
    // Passing bytes straight to bash (normal command)
    Passthrough,
    // User started with '>' — collecting natural language input
    CollectingNl(Vec<u8>),
    // Ollama returned a command — waiting for y/n
    Confirming(String),
}

// ── Entry point ──────────────────────────────────────────────────────────────

pub fn run(config: &Config, db: &Db) -> io::Result<()> {
    let session = uuid();

    let pty_system = native_pty_system();
    let size = crossterm::terminal::size().unwrap_or((80, 24));

    let pair = pty_system.openpty(PtySize {
        rows: size.1, cols: size.0,
        pixel_width: 0, pixel_height: 0,
    }).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let mut cmd = CommandBuilder::new(&config.underlying_shell);
    cmd.env("PSH_SESSION", &session);

    let _child = pair.slave.spawn_command(cmd)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let mut pty_reader = pair.master.try_clone_reader()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let pty_writer = Arc::new(Mutex::new(
        pair.master.take_writer()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
    ));

    // ── Thread: bash output → terminal ───────────────────────────────────────
    // Raw chunk reads so prompts (no trailing newline) render immediately.
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match pty_reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    io::stdout().write_all(&buf[..n]).ok();
                    io::stdout().flush().ok();
                }
            }
        }
    });

    // ── Main input loop ──────────────────────────────────────────────────────
    enable_raw_mode()?;

    let mut mode = Mode::Idle;
    let stdin = io::stdin();
    let mut lock = stdin.lock();   // hold once — no nested lock contention
    let mut buf = [0u8; 1];

    'main: loop {
        if lock.read(&mut buf).unwrap_or(0) == 0 { break; }
        let b = buf[0];

        // Ctrl+D always exits
        if b == 4 { break; }

        mode = match mode {

            // ── IDLE: first byte of a new line ───────────────────────────────
            Mode::Idle => {
                match b {
                    b'>' => {
                        // Switch to NL collection mode — echo the '>'
                        print!(">");
                        io::stdout().flush().ok();
                        Mode::CollectingNl(Vec::new())
                    }
                    b'\r' | b'\n' => {
                        // Empty enter — pass to bash
                        pty_writer.lock().unwrap().write_all(&[b'\r']).ok();
                        Mode::Idle
                    }
                    3 /* Ctrl+C */ => {
                        pty_writer.lock().unwrap().write_all(&[b]).ok();
                        Mode::Idle
                    }
                    _ => {
                        // Normal command — pass this first byte and switch to passthrough
                        pty_writer.lock().unwrap().write_all(&[b]).ok();
                        Mode::Passthrough
                    }
                }
            }

            // ── PASSTHROUGH: normal bash command in progress ─────────────────
            // Write everything to bash. Bash echoes it back through PTY.
            Mode::Passthrough => {
                pty_writer.lock().unwrap().write_all(&[b]).ok();
                match b {
                    b'\r' | b'\n' => Mode::Idle,
                    _ => Mode::Passthrough,
                }
            }

            // ── COLLECTING NL: building the natural language input ────────────
            // We echo manually. Nothing goes to bash.
            Mode::CollectingNl(mut nl_buf) => {
                match b {
                    b'\r' | b'\n' => {
                        let input = String::from_utf8_lossy(&nl_buf).trim().to_string();
                        if input.is_empty() {
                            print!("\r\n");
                            io::stdout().flush().ok();
                            Mode::Idle
                        } else {
                            print!("\r\n\x1b[2mpsh: thinking...\x1b[0m\r\n");
                            io::stdout().flush().ok();

                            match ai::translate_nl(config, db, &session, &input) {
                                Some(cmd) if cmd.starts_with("WARN:") => {
                                    print!("\x1b[33mpsh warning:\x1b[0m {}\r\n",
                                        &cmd[5..].trim());
                                    io::stdout().flush().ok();
                                    Mode::Idle
                                }
                                Some(cmd) => {
                                    print!("\x1b[36mpsh:\x1b[0m {} \x1b[2m[y/n]\x1b[0m ",
                                        cmd);
                                    io::stdout().flush().ok();
                                    if config.confirm_commands {
                                        Mode::Confirming(cmd)
                                    } else {
                                        // Auto-run without confirm
                                        run_command(&pty_writer, &cmd, &session, db);
                                        Mode::Idle
                                    }
                                }
                                None => {
                                    print!("\x1b[31mpsh:\x1b[0m Ollama not reachable.\r\n");
                                    io::stdout().flush().ok();
                                    Mode::Idle
                                }
                            }
                        }
                    }

                    // Backspace
                    0x7f | 0x08 => {
                        if nl_buf.pop().is_some() {
                            print!("\x08 \x08");
                            io::stdout().flush().ok();
                        }
                        Mode::CollectingNl(nl_buf)
                    }

                    // Ctrl+C — cancel NL input
                    3 => {
                        print!("^C\r\n");
                        io::stdout().flush().ok();
                        Mode::Idle
                    }

                    // Printable — buffer and echo
                    _ => {
                        nl_buf.push(b);
                        io::stdout().write_all(&[b]).ok();
                        io::stdout().flush().ok();
                        Mode::CollectingNl(nl_buf)
                    }
                }
            }

            // ── CONFIRMING: waiting for y/n ───────────────────────────────────
            // One keypress. No bash involved.
            Mode::Confirming(cmd) => {
                match b {
                    b'y' | b'Y' | b'\r' | b'\n' => {
                        print!("y\r\n");
                        io::stdout().flush().ok();
                        run_command(&pty_writer, &cmd, &session, db);
                        Mode::Idle
                    }
                    b'n' | b'N' | 3 /* Ctrl+C */ => {
                        print!("n\r\ncancelled\r\n");
                        io::stdout().flush().ok();
                        Mode::Idle
                    }
                    _ => Mode::Confirming(cmd), // any other key — keep waiting
                }
            }
        };
    }

    disable_raw_mode().ok();
    Ok(())
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn run_command(
    pty_writer: &Arc<Mutex<Box<dyn Write + Send>>>,
    cmd: &str,
    session: &str,
    db: &Db,
) {
    let to_run = format!("{}\r\n", cmd);
    pty_writer.lock().unwrap().write_all(to_run.as_bytes()).ok();

    // Brief pause so output settles before we store it
    thread::sleep(std::time::Duration::from_millis(300));

    db.insert(session, &Entry {
        cwd: std::env::current_dir()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        command: cmd.to_string(),
        output: String::new(),
        exit_code: 0,
    });
}

fn uuid() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    format!("{}-{}", t.as_secs(), t.subsec_nanos())
}
