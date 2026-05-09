use crate::config::Config;
use crate::context::{History, HistoryEntry};
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
    // Shadow buffer: mirrors what was typed and forwarded to bash.
    // Ctrl+J submits it as an NL query; regular Enter runs it as a command.
    Passthrough(Vec<u8>),
    // buffer + history_idx (0 = fresh, n = showing nl_prompts()[n-1])
    CollectingNl(Vec<u8>, usize),
    // Receiver + the NL prompt that triggered it
    Thinking(mpsc::Receiver<Option<String>>, String),
    // cmd + the NL prompt that generated it
    Confirming(String, String),
}

// ── Entry point ──────────────────────────────────────────────────────────────

pub fn run(config: &Config, db: &History) -> io::Result<()> {

    let pty_system = native_pty_system();
    let size = crossterm::terminal::size().unwrap_or((80, 24));

    let pair = pty_system.openpty(PtySize {
        rows: size.1, cols: size.0,
        pixel_width: 0, pixel_height: 0,
    }).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let cmd = CommandBuilder::new(&config.underlying_shell);

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

    // Stdin reader thread
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

    // Drain terminal init sequences — GNOME Terminal sends more when it's the default shell
    thread::sleep(Duration::from_millis(200));
    while key_rx.try_recv().is_ok() {}

    let spinner_stop = Arc::new(AtomicBool::new(true));
    let mut mode = Mode::Idle;

    'main: loop {

        // ── THINKING: non-blocking — check ESC then check AI result ──────────
        if matches!(mode, Mode::Thinking(..)) {
            let (rx, nl_prompt) = match std::mem::replace(&mut mode, Mode::Idle) {
                Mode::Thinking(rx, p) => (rx, p),
                _ => unreachable!(),
            };

            let mut cancelled = false;
            while let Ok(b) = key_rx.try_recv() {
                if b == 0x1b || b == 3 { cancelled = true; break; }
            }
            if cancelled {
                while key_rx.try_recv().is_ok() {}
                stop_spinner(&spinner_stop);
                pty_writer.lock().unwrap().write_all(b"\r").ok();
                continue 'main;
            }

            match rx.try_recv() {
                Ok(result) => {
                    while key_rx.try_recv().is_ok() {}
                    stop_spinner(&spinner_stop);
                    mode = show_result(result, &nl_prompt, config, &pty_writer, db);
                }
                Err(mpsc::TryRecvError::Empty) => {
                    mode = Mode::Thinking(rx, nl_prompt);
                    thread::sleep(Duration::from_millis(10));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    while key_rx.try_recv().is_ok() {}
                    stop_spinner(&spinner_stop);
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
                b'\r' | b'\n' => {
                    pty_writer.lock().unwrap().write_all(&[b'\r']).ok();
                    Mode::Idle
                }
                3 => {
                    pty_writer.lock().unwrap().write_all(&[b]).ok();
                    Mode::Idle
                }
                0x1b => {
                    let mut rest: Vec<u8> = Vec::new();
                    let deadline = std::time::Instant::now() + Duration::from_millis(20);
                    loop {
                        if std::time::Instant::now() >= deadline { break; }
                        match key_rx.try_recv() {
                            Ok(c) => {
                                rest.push(c);
                                if rest.first() == Some(&b'[') {
                                    if c >= 0x40 && c <= 0x7e { break; }
                                } else {
                                    break;
                                }
                            }
                            Err(mpsc::TryRecvError::Empty) => thread::sleep(Duration::from_micros(200)),
                            Err(_) => break,
                        }
                    }
                    match rest.as_slice() {
                        b"\r" => {
                            // Alt+Enter on empty line: enter NL collecting mode
                            Mode::CollectingNl(Vec::new(), 0)
                        }
                        b"[A" | b"[B" | b"[C" | b"[D" => {
                            // Arrow keys: pass to bash readline, shadow starts empty
                            pty_writer.lock().unwrap().write_all(&[0x1b]).ok();
                            pty_writer.lock().unwrap().write_all(&rest).ok();
                            Mode::Passthrough(Vec::new())
                        }
                        _ => {
                            pty_writer.lock().unwrap().write_all(&[0x1b]).ok();
                            if !rest.is_empty() {
                                pty_writer.lock().unwrap().write_all(&rest).ok();
                            }
                            Mode::Idle
                        }
                    }
                }
                _ => {
                    pty_writer.lock().unwrap().write_all(&[b]).ok();
                    Mode::Passthrough(vec![b])
                }
            },

            // ── PASSTHROUGH ──────────────────────────────────────────────────
            // Shadow buffer mirrors what was typed so Ctrl+J can submit it to AI.
            Mode::Passthrough(mut shadow) => match b {
                b'\n' => {
                    // Ctrl+J: submit shadow as NL query
                    let input = String::from_utf8_lossy(&shadow).trim().to_string();
                    if !input.is_empty() {
                        // Move to next line first so typed text stays visible above
                        print!("\r\n");
                        io::stdout().flush().ok();
                        // Clear bash's input buffer (Ctrl+U) — on the new empty line,
                        // its echo has no visible effect on the text above
                        thread::sleep(Duration::from_millis(10));
                        pty_writer.lock().unwrap().write_all(&[b'\x15']).ok();

                        spinner_stop.store(false, Ordering::Relaxed);
                        let stop = spinner_stop.clone();
                        thread::spawn(move || spin(stop));

                        let recent = db.recent(10);
                        let cfg = config.clone();
                        let nl = input.clone();
                        let (tx, rx) = mpsc::channel();
                        thread::spawn(move || {
                            let result = ai::translate_nl(&cfg, &recent, &nl);
                            tx.send(result).ok();
                        });
                        Mode::Thinking(rx, input)
                    } else {
                        pty_writer.lock().unwrap().write_all(&[b'\r']).ok();
                        Mode::Idle
                    }
                }
                0x7f | 0x08 => {
                    shadow.pop();
                    pty_writer.lock().unwrap().write_all(&[b]).ok();
                    Mode::Passthrough(shadow)
                }
                0x15 => {
                    // Ctrl+U: clear shadow too
                    shadow.clear();
                    pty_writer.lock().unwrap().write_all(&[b]).ok();
                    Mode::Passthrough(shadow)
                }
                b'\r' | 3 => {
                    pty_writer.lock().unwrap().write_all(&[b]).ok();
                    Mode::Idle
                }
                0x1b => {
                    // Consume full ESC sequence
                    let mut rest: Vec<u8> = Vec::new();
                    let deadline = std::time::Instant::now() + Duration::from_millis(20);
                    loop {
                        if std::time::Instant::now() >= deadline { break; }
                        match key_rx.try_recv() {
                            Ok(c) => {
                                rest.push(c);
                                if rest.first() == Some(&b'[') {
                                    if c >= 0x40 && c <= 0x7e { break; }
                                } else {
                                    break;
                                }
                            }
                            Err(mpsc::TryRecvError::Empty) => thread::sleep(Duration::from_micros(200)),
                            Err(_) => break,
                        }
                    }
                    if rest.as_slice() == b"\r" {
                        // Alt+Enter: same as Ctrl+J — submit shadow as NL query
                        let input = String::from_utf8_lossy(&shadow).trim().to_string();
                        if !input.is_empty() {
                            print!("\r\n");
                            io::stdout().flush().ok();
                            thread::sleep(Duration::from_millis(10));
                            pty_writer.lock().unwrap().write_all(&[b'\x15']).ok();

                            spinner_stop.store(false, Ordering::Relaxed);
                            let stop = spinner_stop.clone();
                            thread::spawn(move || spin(stop));

                            let recent = db.recent(10);
                            let cfg = config.clone();
                            let nl = input.clone();
                            let (tx, rx) = mpsc::channel();
                            thread::spawn(move || {
                                let result = ai::translate_nl(&cfg, &recent, &nl);
                                tx.send(result).ok();
                            });
                            Mode::Thinking(rx, input)
                        } else {
                            // Nothing typed yet: enter fresh NL collecting mode
                            Mode::CollectingNl(Vec::new(), 0)
                        }
                    } else {
                        // Other ESC sequence (arrows, terminal queries): forward, shadow unchanged
                        pty_writer.lock().unwrap().write_all(&[0x1b]).ok();
                        if !rest.is_empty() {
                            pty_writer.lock().unwrap().write_all(&rest).ok();
                        }
                        Mode::Passthrough(shadow)
                    }
                }
                _ => {
                    shadow.push(b);
                    pty_writer.lock().unwrap().write_all(&[b]).ok();
                    Mode::Passthrough(shadow)
                }
            },

            // ── COLLECTING NL ────────────────────────────────────────────────
            Mode::CollectingNl(mut nl_buf, hist_idx) => match b {
                b'\r' | b'\n' => {
                    let input = String::from_utf8_lossy(&nl_buf).trim().to_string();
                    if input.is_empty() {
                        print!("\r\n");
                        io::stdout().flush().ok();
                        Mode::Idle
                    } else {
                        print!("\r\n");
                        io::stdout().flush().ok();

                        spinner_stop.store(false, Ordering::Relaxed);
                        let stop = spinner_stop.clone();
                        thread::spawn(move || spin(stop));

                        let recent = db.recent(10);
                        let cfg = config.clone();
                        let nl = input.clone();
                        let (tx, rx) = mpsc::channel();
                        thread::spawn(move || {
                            let result = ai::translate_nl(&cfg, &recent, &nl);
                            tx.send(result).ok();
                        });

                        Mode::Thinking(rx, input)
                    }
                }
                0x7f | 0x08 => {
                    if nl_buf.pop().is_some() {
                        print!("\x08 \x08");
                        io::stdout().flush().ok();
                    }
                    Mode::CollectingNl(nl_buf, hist_idx)
                }
                3 => {
                    print!("\r\n");
                    io::stdout().flush().ok();
                    pty_writer.lock().unwrap().write_all(b"\r").ok();
                    Mode::Idle
                }
                0x1b => {
                    thread::sleep(Duration::from_micros(500));
                    match key_rx.try_recv() {
                        Ok(b'[') => {
                            let mut seq = vec![b'['];
                            let deadline = std::time::Instant::now() + Duration::from_millis(20);
                            loop {
                                if std::time::Instant::now() >= deadline { break; }
                                match key_rx.try_recv() {
                                    Ok(c) => { seq.push(c); if c >= 0x40 && c <= 0x7e { break; } }
                                    Err(mpsc::TryRecvError::Empty) => thread::sleep(Duration::from_micros(200)),
                                    Err(_) => break,
                                }
                            }
                            match seq.as_slice() {
                                b"[A" => {
                                    let prompts = db.nl_prompts();
                                    let new_idx = hist_idx + 1;
                                    if new_idx <= prompts.len() {
                                        let p = prompts[new_idx - 1].clone();
                                        for _ in 0..nl_buf.len() { print!("\x08 \x08"); }
                                        print!("{}", p);
                                        io::stdout().flush().ok();
                                        Mode::CollectingNl(p.into_bytes(), new_idx)
                                    } else {
                                        Mode::CollectingNl(nl_buf, hist_idx)
                                    }
                                }
                                b"[B" => {
                                    if hist_idx == 0 {
                                        Mode::CollectingNl(nl_buf, 0)
                                    } else {
                                        let new_idx = hist_idx - 1;
                                        for _ in 0..nl_buf.len() { print!("\x08 \x08"); }
                                        if new_idx == 0 {
                                            io::stdout().flush().ok();
                                            Mode::CollectingNl(vec![], 0)
                                        } else {
                                            let prompts = db.nl_prompts();
                                            let p = prompts[new_idx - 1].clone();
                                            print!("{}", p);
                                            io::stdout().flush().ok();
                                            Mode::CollectingNl(p.into_bytes(), new_idx)
                                        }
                                    }
                                }
                                _ => {
                                    pty_writer.lock().unwrap().write_all(&[0x1b]).ok();
                                    pty_writer.lock().unwrap().write_all(&seq).ok();
                                    Mode::CollectingNl(nl_buf, hist_idx)
                                }
                            }
                        }
                        Ok(b'\r') => {
                            // Alt+Enter while collecting: submit
                            let input = String::from_utf8_lossy(&nl_buf).trim().to_string();
                            print!("\r\n");
                            io::stdout().flush().ok();
                            if input.is_empty() {
                                pty_writer.lock().unwrap().write_all(b"\r").ok();
                                Mode::Idle
                            } else {
                                spinner_stop.store(false, Ordering::Relaxed);
                                let stop = spinner_stop.clone();
                                thread::spawn(move || spin(stop));
                                let recent = db.recent(10);
                                let cfg = config.clone();
                                let nl = input.clone();
                                let (tx, rx) = mpsc::channel();
                                thread::spawn(move || {
                                    let result = ai::translate_nl(&cfg, &recent, &nl);
                                    tx.send(result).ok();
                                });
                                Mode::Thinking(rx, input)
                            }
                        }
                        Ok(other) => {
                            print!("\r\n");
                            io::stdout().flush().ok();
                            pty_writer.lock().unwrap().write_all(&[0x1b, other]).ok();
                            Mode::Idle
                        }
                        Err(_) => {
                            // ESC alone: cancel
                            print!("\r\n");
                            io::stdout().flush().ok();
                            pty_writer.lock().unwrap().write_all(b"\r").ok();
                            Mode::Idle
                        }
                    }
                }
                _ => {
                    nl_buf.push(b);
                    io::stdout().write_all(&[b]).ok();
                    io::stdout().flush().ok();
                    Mode::CollectingNl(nl_buf, 0)
                }
            },

            // ── CONFIRMING ───────────────────────────────────────────────────
            Mode::Confirming(cmd, nl_prompt) => match b {
                b'y' | b'Y' | b'\r' | b'\n' => {
                    print!("y\r\n");
                    io::stdout().flush().ok();
                    run_command(&pty_writer, &cmd, &nl_prompt, db);
                    Mode::Idle
                }
                b'n' | b'N' | 3 | 0x1b => {
                    print!("n\r\n");
                    io::stdout().flush().ok();
                    pty_writer.lock().unwrap().write_all(b"\r").ok();
                    Mode::Idle
                }
                _ => Mode::Confirming(cmd, nl_prompt),
            },

            Mode::Thinking(_, _) => unreachable!(),
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
    thread::sleep(Duration::from_millis(90));
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn show_result(
    result: Option<String>,
    nl_prompt: &str,
    config: &Config,
    pty_writer: &Arc<Mutex<Box<dyn Write + Send>>>,
    db: &History,
) -> Mode {
    match result {
        Some(resp) if resp.starts_with("ANSWER:") => {
            print!("\r\x1b[2K\x1b[2m  {}\x1b[0m\r\n", resp[7..].trim());
            io::stdout().flush().ok();
            pty_writer.lock().unwrap().write_all(b"\r").ok();
            Mode::Idle
        }
        Some(resp) if resp.starts_with("WARN:") => {
            print!("\r\x1b[2K\x1b[33m  ⚠  {}\x1b[0m\r\n", resp[5..].trim());
            io::stdout().flush().ok();
            pty_writer.lock().unwrap().write_all(b"\r").ok();
            Mode::Idle
        }
        Some(resp) => {
            let cmd = resp.strip_prefix("CMD:").unwrap_or(&resp).trim().to_string();
            let auto_run = is_safe_command(&cmd) || !config.confirm_commands;

            print!("\r\x1b[2K");
            if cmd.contains(" && ") {
                for (i, step) in cmd.split(" && ").enumerate() {
                    if i == 0 {
                        print!("\x1b[36m  ❯\x1b[0m  {}", step.trim());
                    } else {
                        print!("\r\n     \x1b[2m&&\x1b[0m  {}", step.trim());
                    }
                }
            } else {
                print!("\x1b[36m  ❯\x1b[0m  {}", cmd);
            }

            if auto_run {
                print!("\r\n");
                io::stdout().flush().ok();
                run_command(pty_writer, &cmd, nl_prompt, db);
                Mode::Idle
            } else {
                print!("  \x1b[2m[y/n]\x1b[0m ");
                io::stdout().flush().ok();
                Mode::Confirming(cmd, nl_prompt.to_string())
            }
        }
        None => {
            print!("\r\x1b[2K\x1b[31m  ✗  Ollama not reachable.\x1b[0m\r\n");
            io::stdout().flush().ok();
            Mode::Idle
        }
    }
}

fn is_safe_command(cmd: &str) -> bool {
    cmd.split("&&").all(|part| {
        let first = part.trim().split_whitespace().next().unwrap_or("");
        match first {
            "ls" | "find" | "grep" | "egrep" | "fgrep" | "rg" | "ag" | "fd" |
            "cat" | "head" | "tail" | "less" | "more" | "wc" | "sort" | "uniq" |
            "cut" | "awk" | "sed" | "echo" | "printf" | "pwd" | "which" | "type" |
            "file" | "stat" | "du" | "df" | "ps" | "free" | "uname" | "hostname" |
            "whoami" | "id" | "groups" | "diff" | "locate" | "tree" => true,
            "git" => matches!(
                part.trim().split_whitespace().nth(1).unwrap_or(""),
                "status" | "log" | "diff" | "branch" | "show" |
                "remote" | "ls-files" | "describe" | "tag" | "stash"
            ),
            _ => false,
        }
    })
}

fn run_command(
    pty_writer: &Arc<Mutex<Box<dyn Write + Send>>>,
    cmd: &str,
    nl_prompt: &str,
    db: &History,
) {
    pty_writer.lock().unwrap().write_all(format!("{}\r\n", cmd).as_bytes()).ok();
    thread::sleep(Duration::from_millis(300));
    db.append(&HistoryEntry {
        cwd:       std::env::current_dir().unwrap_or_default().to_string_lossy().to_string(),
        nl_prompt: if nl_prompt.is_empty() { None } else { Some(nl_prompt.to_string()) },
        command:   cmd.to_string(),
        exit_code: 0,
    });
}
