use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::thread;

/// Owns the shell subprocess and the PTY master.
/// A background thread continuously reads PTY output and sends it over a channel
/// so the main loop can drain it without blocking the render loop.
pub struct PtySession {
    pub output: Receiver<Vec<u8>>,
    /// PID of the shell process — used to detect what is running in the PTY.
    pub shell_pid: Option<u32>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    _child: Box<dyn Child + Send + Sync>,
}

impl PtySession {
    pub fn new(rows: u16, cols: u16, cwd: Option<&str>) -> Result<Self> {
        let pty_system = native_pty_system();

        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        // Spawn $SHELL (falls back to bash)
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "bash".into());
        let mut cmd = CommandBuilder::new(shell);
        if let Some(dir) = cwd {
            cmd.cwd(dir);
        }
        let child = pair.slave.spawn_command(cmd)?;

        // Clone reader before taking the writer — both borrow master separately
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        let (tx, rx) = mpsc::channel();

        // Background thread: read PTY output → send to main loop
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        let shell_pid = child.process_id();
        Ok(Self {
            output: rx,
            shell_pid,
            master: pair.master,
            writer,
            _child: child,
        })
    }

    /// Notify the shell of a terminal resize.
    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }

    /// Write raw bytes directly to the PTY (e.g. clipboard paste text).
    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer.write_all(bytes)?;
        Ok(())
    }

    /// Encode a crossterm KeyEvent and write it to the PTY.
    pub fn write_key(&mut self, key: KeyEvent) -> Result<()> {
        let bytes: Vec<u8> = match (key.modifiers, key.code) {
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char(c)) => {
                c.to_string().into_bytes()
            }
            (KeyModifiers::CONTROL, KeyCode::Char(c)) => {
                // Ctrl+A = 0x01, Ctrl+B = 0x02, ...
                vec![(c as u8).to_ascii_lowercase() & 0x1f]
            }
            (_, KeyCode::Enter) => vec![b'\r'],
            (_, KeyCode::Backspace) => vec![0x7f],
            (_, KeyCode::Delete) => b"\x1b[3~".to_vec(),
            (_, KeyCode::Tab) => vec![b'\t'],
            (_, KeyCode::Esc) => vec![0x1b],
            (_, KeyCode::Up) => b"\x1b[A".to_vec(),
            (_, KeyCode::Down) => b"\x1b[B".to_vec(),
            (KeyModifiers::CONTROL, KeyCode::Right) => b"\x1b[1;5C".to_vec(),
            (KeyModifiers::CONTROL, KeyCode::Left)  => b"\x1b[1;5D".to_vec(),
            (_, KeyCode::Right) => b"\x1b[C".to_vec(),
            (_, KeyCode::Left) => b"\x1b[D".to_vec(),
            (_, KeyCode::Home) => b"\x1b[H".to_vec(),
            (_, KeyCode::End) => b"\x1b[F".to_vec(),
            (_, KeyCode::PageUp) => b"\x1b[5~".to_vec(),
            (_, KeyCode::PageDown) => b"\x1b[6~".to_vec(),
            _ => return Ok(()),
        };

        self.writer.write_all(&bytes)?;
        Ok(())
    }
}
