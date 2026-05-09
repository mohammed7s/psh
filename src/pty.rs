use crate::config::Config;
use crate::context::{Db, Entry};
use crate::ai;

use crossterm::terminal::{enable_raw_mode, disable_raw_mode};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

// ── State machine ────────────────────────────────────────────────────────────

enum Mode {
    Idle,
    Passthrough,
    CollectingNl(Vec<u8>),
    // Ollama running on background thread — main loop stays free to read ESC
    Thinking(mpsc::Receiver<Option<String>>),
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

    // Thread: bash output → terminal
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

    enable_raw_mode()?;

    // Stdin reader thread — frees the main loop from blocking reads
    // so ESC/Ctrl+C can always be received even during Ollama calls
    let (key_tx, key_rx) = mpsc::channel::<u8>();
    thread::spawn(move || {
        let stdin = io::stdin();
        let mut lock = stdin.lock();
        let mut buf = [0u8; 1];
        loop {
            if lock.read(&mut buf).unwrap_or(0) == 0 { break; }
            if key_tx.send(buf[0]).is_err() { break; }
        }
    });

    // Drain terminal init sequences (e.g. \x1b[?...) sent right after raw mode starts
    thread::sleep(Duration::from_millis(30));
    while key_rx.try_recv().is_ok() {}

    let spinner_stop = Arc::new(AtomicBool::new(true));
    let mut mode = Mode::Idle;

    'main: loop {

        // ── THINKING: non-blocking — check ESC then check AI result ──────────
        if matches!(mode, Mode::Thinking(_)) {
            // Pull rx out so we can reassign mode freely
            let rx = match std::mem::replace(&mut mode, Mode::Idle) {
                Mode::Thinking(rx) => rx,
                _ => unreachable!(),
            };

            // Drain ALL pending keys — only ESC/Ctrl+C cancels, rest are discarded.
            // Keys typed during thinking must not leak into the next state.
            let mut cancelled = false;
            while let Ok(b) = key_rx.try_recv() {
                if b == 0x1b || b == 3 {
                    cancelled = true;
                    break;
                }
            }
            if cancelled {
                while key_rx.try_recv().is_ok() {} // flush remainder
                stop_spinner(&spinner_stop);
                pty_writer.lock().unwrap().write_all(b"\r").ok();
                continue 'main;
            }

            match rx.try_recv() {
                Ok(result) => {
                    while key_rx.try_recv().is_ok() {} // flush stale input before next state
                    stop_spinner(&spinner_stop);
                    mode = show_result(result, config, &pty_writer, &session, db);
                }
                Err(mpsc::TryRecvError::Empty) => {
                    mode = Mode::Thinking(rx); // still waiting
                    thread::sleep(Duration::from_millis(10));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    while key_rx.try_recv().is_ok() {}
                    stop_spinner(&spinner_stop);
                    // mode stays Idle
                }
            }
            continue 'main;
        }

        // ── All other states: block until a key arrives ───────────────────────
        let b = match key_rx.recv() {
            Ok(b) => b,
            Err(_) => break 'main,
        };

        if b == 4 { break 'main; } // Ctrl+D

        mode = match mode {

            // ── IDLE ─────────────────────────────────────────────────────────
            Mode::Idle => match b {
                b' ' => {
                    print!(" ");
                    io::stdout().flush().ok();
                    Mode::CollectingNl(Vec::new())
                }
                b'\r' | b'\n' => {
                    pty_writer.lock().unwrap().write_all(&[b'\r']).ok();
                    Mode::Idle
                }
                3 => {
                    pty_writer.lock().unwrap().write_all(&[b]).ok();
                    Mode::Idle
                }
                0x1b => {
                    // ESC in Idle: could be user ESC or a terminal escape sequence
                    // response (e.g. \x1b[24;1R from cursor-position query).
                    // Consume the entire sequence so follow-on bytes don't land in
                    // the Idle match and trigger Passthrough.
                    pty_writer.lock().unwrap().write_all(&[b]).ok();
                    let mut rest: Vec<u8> = Vec::new();
                    let deadline = std::time::Instant::now() + Duration::from_millis(5);
                    loop {
                        if std::time::Instant::now() >= deadline { break; }
                        match key_rx.try_recv() {
                            Ok(c) => {
                                rest.push(c);
                                // CSI sequence (\x1b[...): final byte is 0x40-0x7E
                                if rest.first() == Some(&b'[') {
                                    if c >= 0x40 && c <= 0x7e { break; }
                                } else {
                                    break; // single byte after ESC (e.g. \x1bO, \x1bA)
                                }
                            }
                            Err(mpsc::TryRecvError::Empty) => {
                                thread::sleep(Duration::from_micros(200));
                            }
                            Err(_) => break,
                        }
                    }
                    if !rest.is_empty() {
                        pty_writer.lock().unwrap().write_all(&rest).ok();
                    }
                    Mode::Idle
                }
                _ => {
                    pty_writer.lock().unwrap().write_all(&[b]).ok();
                    Mode::Passthrough
                }
            },

            // ── PASSTHROUGH ──────────────────────────────────────────────────
            Mode::Passthrough => {
                pty_writer.lock().unwrap().write_all(&[b]).ok();
                match b {
                    b'\r' | b'\n' | 3 => Mode::Idle,
                    _ => Mode::Passthrough,
                }
            }

            // ── COLLECTING NL ────────────────────────────────────────────────
            Mode::CollectingNl(mut nl_buf) => match b {
                b'\r' | b'\n' => {
                    let input = String::from_utf8_lossy(&nl_buf).trim().to_string();
                    if input.is_empty() {
                        print!("\r\n");
                        io::stdout().flush().ok();
                        Mode::Idle
                    } else {
                        print!("\r\n");
                        io::stdout().flush().ok();

                        // Start spinner
                        spinner_stop.store(false, Ordering::Relaxed);
                        let stop = spinner_stop.clone();
                        thread::spawn(move || spin(stop));

                        // Ollama on background thread — main loop stays responsive
                        let recent = db.recent(&session, 10);
                        let cfg = config.clone();
                        let (tx, rx) = mpsc::channel();
                        thread::spawn(move || {
                            let result = ai::translate_nl(&cfg, &recent, &input);
                            tx.send(result).ok();
                        });

                        Mode::Thinking(rx)
                    }
                }
                0x7f | 0x08 => {
                    if nl_buf.pop().is_some() {
                        print!("\x08 \x08");
                        io::stdout().flush().ok();
                    }
                    Mode::CollectingNl(nl_buf)
                }
                0x1b | 3 => {
                    print!("\r\n");
                    io::stdout().flush().ok();
                    pty_writer.lock().unwrap().write_all(b"\r").ok();
                    Mode::Idle
                }
                _ => {
                    nl_buf.push(b);
                    io::stdout().write_all(&[b]).ok();
                    io::stdout().flush().ok();
                    Mode::CollectingNl(nl_buf)
                }
            },

            // ── CONFIRMING ───────────────────────────────────────────────────
            Mode::Confirming(cmd) => match b {
                b'y' | b'Y' | b'\r' | b'\n' => {
                    print!("y\r\n");
                    io::stdout().flush().ok();
                    run_command(&pty_writer, &cmd, &session, db);
                    Mode::Idle
                }
                b'n' | b'N' | 3 | 0x1b => {
                    print!("n\r\n");
                    io::stdout().flush().ok();
                    pty_writer.lock().unwrap().write_all(b"\r").ok();
                    Mode::Idle
                }
                _ => Mode::Confirming(cmd),
            },

            Mode::Thinking(_) => unreachable!(),
        };
    }

    disable_raw_mode().ok();
    Ok(())
}

// ── Spinner ──────────────────────────────────────────────────────────────────

fn spin(stop: Arc<AtomicBool>) {
    let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let mut i = 0usize;
    while !stop.load(Ordering::Relaxed) {
        print!("\r  \x1b[36m{}\x1b[0m  \x1b[2mthinking\x1b[0m ", frames[i % frames.len()]);
        io::stdout().flush().ok();
        i += 1;
        thread::sleep(Duration::from_millis(80));
    }
    print!("\r\x1b[2K");
    io::stdout().flush().ok();
}

fn stop_spinner(stop: &Arc<AtomicBool>) {
    stop.store(true, Ordering::Relaxed);
    thread::sleep(Duration::from_millis(90)); // wait for spinner thread to clear the line
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn show_result(
    result: Option<String>,
    config: &Config,
    pty_writer: &Arc<Mutex<Box<dyn Write + Send>>>,
    session: &str,
    db: &Db,
) -> Mode {
    match result {
        Some(resp) if resp.starts_with("ANSWER:") => {
            print!("\r\x1b[2K\x1b[2m  {}\x1b[0m\r\n", resp[7..].trim());
            io::stdout().flush().ok();
            Mode::Idle
        }
        Some(resp) if resp.starts_with("WARN:") => {
            print!("\r\x1b[2K\x1b[33m  ⚠  {}\x1b[0m\r\n", resp[5..].trim());
            io::stdout().flush().ok();
            Mode::Idle
        }
        Some(resp) => {
            let cmd = resp.strip_prefix("CMD:").unwrap_or(&resp).trim().to_string();
            print!("\r\x1b[2K\x1b[36m  ❯\x1b[0m  {}  \x1b[2m[y/n]\x1b[0m ", cmd);
            io::stdout().flush().ok();
            if config.confirm_commands {
                Mode::Confirming(cmd)
            } else {
                run_command(pty_writer, &cmd, session, db);
                Mode::Idle
            }
        }
        None => {
            print!("\r\x1b[2K\x1b[31m  ✗  Ollama not reachable.\x1b[0m\r\n");
            io::stdout().flush().ok();
            Mode::Idle
        }
    }
}

fn run_command(
    pty_writer: &Arc<Mutex<Box<dyn Write + Send>>>,
    cmd: &str,
    session: &str,
    db: &Db,
) {
    pty_writer.lock().unwrap().write_all(format!("{}\r\n", cmd).as_bytes()).ok();
    thread::sleep(Duration::from_millis(300));
    db.insert(session, &Entry {
        cwd:      std::env::current_dir().unwrap_or_default().to_string_lossy().to_string(),
        command:  cmd.to_string(),
        output:   String::new(),
        exit_code: 0,
    });
}

fn uuid() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    format!("{}-{}", t.as_secs(), t.subsec_nanos())
}
