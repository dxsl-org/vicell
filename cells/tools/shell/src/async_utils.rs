use ostd::executor::yield_now;
use ostd::input::{poll_events, InputEvent, KeyState, KeySym};
use ostd::prelude::*;
use api::syscall::service;

pub struct AsyncStdin;

impl AsyncStdin {
    /// Read a line from stdin into an owned Vec<u8>.
    ///
    /// Sources in priority order:
    ///   1. VirtIO keyboard + UART via input service (EV_ASCII relay)
    ///   2. UART/serial via sys_read(fd=0) — fallback when input service absent
    ///
    /// Returns the bytes entered (excluding the newline). Ownership satisfies
    /// Law 2: no borrowed slice across `.await`.
    pub async fn read_line(
        &self,
        max_len: usize,
        history: &mut alloc::collections::VecDeque<alloc::string::String>,
    ) -> Vec<u8> {
        let mut buffer: Vec<u8> = Vec::with_capacity(max_len);
        let mut history_idx = history.len();
        let mut escape_state: u8 = 0; // ANSI state machine — UART path only

        'read: loop {
            if buffer.len() >= max_len {
                break;
            }

            // ── Input service path (VirtIO keyboard → input service → shell) ──
            // KeySym is already decoded; no escape-state-machine needed here.
            let events = poll_events(8);
            let got_event = !events.is_empty();

            for ev in events {
                let InputEvent::Key(k) = ev else { continue };
                if !matches!(k.state, KeyState::Pressed | KeyState::Repeated) {
                    continue;
                }
                match k.keysym {
                    KeySym::Return => {
                        ostd::io::print("\n");
                        break 'read;
                    }
                    KeySym::Backspace => {
                        if !buffer.is_empty() {
                            ostd::io::print("\x08 \x08");
                            buffer.pop();
                        }
                    }
                    KeySym::Tab => {
                        Self::handle_tab(&mut buffer);
                    }
                    KeySym::Up => {
                        if history_idx > 0 {
                            history_idx -= 1;
                            Self::clear_line(&buffer);
                            buffer.clear();
                            if let Some(cmd) = history.get(history_idx) {
                                ostd::io::print(cmd);
                                buffer.extend_from_slice(cmd.as_bytes());
                            }
                        }
                    }
                    KeySym::Down => {
                        if history_idx < history.len() {
                            history_idx += 1;
                            Self::clear_line(&buffer);
                            buffer.clear();
                            if history_idx < history.len() {
                                if let Some(cmd) = history.get(history_idx) {
                                    ostd::io::print(cmd);
                                    buffer.extend_from_slice(cmd.as_bytes());
                                }
                            }
                        }
                    }
                    _ => {
                        // Printable ASCII only — multi-byte UTF-8 deferred.
                        if let Some(ch) = k.char() {
                            let cp = ch as u32;
                            if cp >= 0x20 && cp <= 0x7E && buffer.len() < max_len {
                                let byte = ch as u8;
                                if let Ok(s) = core::str::from_utf8(core::slice::from_ref(&byte)) {
                                    ostd::io::print(s);
                                }
                                buffer.push(byte);
                            }
                        }
                    }
                }
            }

            if got_event {
                // Processed at least one input-service event; re-check before
                // falling through to UART so key bursts don't stall.
                continue;
            }

            // When the input service is registered it delivers UART chars via
            // EV_ASCII IPC *and* the kernel also stores them in the UART ring
            // buffer.  Reading both paths doubles every keystroke.  Skip UART
            // entirely while the input service is online.
            if ostd::syscall::sys_lookup_service(service::INPUT).is_some() {
                yield_now().await;
                continue;
            }

            // ── UART / serial fallback (headless: no VirtIO keyboard) ─────
            let mut c = [0u8; 1];
            match ostd::syscall::sys_read(0, &mut c) {
                Ok(n) if n > 0 => {
                    let ch = c[0];

                    // ANSI escape sequence state machine
                    if escape_state == 0 {
                        if ch == 0x1B {
                            escape_state = 1;
                            continue;
                        }
                    } else if escape_state == 1 {
                        escape_state = if ch == b'[' { 2 } else { 0 };
                        continue;
                    } else if escape_state == 2 {
                        escape_state = 0;
                        match ch {
                            b'A' => {
                                if history_idx > 0 {
                                    history_idx -= 1;
                                    Self::clear_line(&buffer);
                                    buffer.clear();
                                    if let Some(cmd) = history.get(history_idx) {
                                        ostd::io::print(cmd);
                                        buffer.extend_from_slice(cmd.as_bytes());
                                    }
                                }
                            }
                            b'B' => {
                                if history_idx < history.len() {
                                    history_idx += 1;
                                    Self::clear_line(&buffer);
                                    buffer.clear();
                                    if let Some(cmd) = history.get(history_idx) {
                                        ostd::io::print(cmd);
                                        buffer.extend_from_slice(cmd.as_bytes());
                                    }
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }

                    if ch == 0x09 {
                        Self::handle_tab(&mut buffer);
                        continue;
                    }
                    if ch == b'\r' || ch == b'\n' {
                        ostd::io::print("\n");
                        break;
                    }
                    if ch == 8 || ch == 127 {
                        if !buffer.is_empty() {
                            ostd::io::print("\x08 \x08");
                            buffer.pop();
                        }
                        continue;
                    }
                    if let Ok(s) = core::str::from_utf8(&c) {
                        ostd::io::print(s);
                    }
                    buffer.push(ch);
                }
                _ => {
                    yield_now().await;
                }
            }
        }
        buffer
    }

    /// TAB completion: complete the last token against built-in command names.
    fn handle_tab(buffer: &mut Vec<u8>) {
        let line = core::str::from_utf8(buffer).unwrap_or("");
        let token = line.split_whitespace().last().unwrap_or("");
        let token_bytes = token.len();

        let matches: alloc::vec::Vec<&str> = crate::executor::BUILTINS
            .iter()
            .filter(|b| b.starts_with(token))
            .copied()
            .collect();

        match matches.len() {
            0 => {}
            1 => {
                for _ in 0..token_bytes {
                    ostd::io::print("\x08 \x08");
                    buffer.pop();
                }
                let completed = matches[0];
                ostd::io::print(completed);
                ostd::io::print(" ");
                buffer.extend_from_slice(completed.as_bytes());
                buffer.push(b' ');
            }
            _ => {
                ostd::io::print("\n");
                for (i, m) in matches.iter().enumerate() {
                    if i > 0 {
                        ostd::io::print("  ");
                    }
                    ostd::io::print(m);
                }
                ostd::io::print("\n");
                if let Ok(s) = core::str::from_utf8(buffer) {
                    ostd::io::print(s);
                }
            }
        }
    }

    fn clear_line(current: &[u8]) {
        for _ in 0..current.len() {
            ostd::io::print("\x08 \x08");
        }
    }
}
