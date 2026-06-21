use ostd::executor::yield_now;
use ostd::prelude::*;

pub struct AsyncStdin;

impl AsyncStdin {
    /// Read a line from stdin into an owned Vec<u8>.
    ///
    /// Returns the bytes entered (excluding the newline). Passing ownership
    /// instead of `&mut [u8]` satisfies Law 2: no borrowed slice across `.await`.
    pub async fn read_line(
        &self,
        max_len: usize,
        history: &mut alloc::collections::VecDeque<alloc::string::String>,
    ) -> Vec<u8> {
        let mut buffer: Vec<u8> = Vec::with_capacity(max_len);
        let mut history_idx = history.len();
        let mut escape_state: u8 = 0; // 0=Normal 1=Esc 2=Bracket

        loop {
            if buffer.len() >= max_len {
                break;
            }
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
                        if ch == b'[' {
                            escape_state = 2;
                        } else {
                            escape_state = 0;
                        }
                        continue;
                    } else if escape_state == 2 {
                        escape_state = 0;
                        match ch {
                            b'A' => {
                                // Up arrow — load previous history entry
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
                                // Down arrow — load next history entry or blank
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

                    // TAB (0x09) — complete the last whitespace-delimited token.
                    if ch == 0x09 {
                        Self::handle_tab(&mut buffer);
                        continue;
                    }

                    // Normal character processing
                    if ch == b'\r' || ch == b'\n' {
                        ostd::io::print("\n");
                        break;
                    }
                    if ch == 8 || ch == 127 {
                        // Backspace
                        if !buffer.is_empty() {
                            ostd::io::print("\x08 \x08");
                            buffer.pop();
                        }
                        continue;
                    }
                    // Echo and append
                    if let Ok(s) = core::str::from_utf8(&c) {
                        ostd::io::print(s);
                    }
                    buffer.push(ch);
                }
                _ => {
                    ostd::executor::sleep(1).await;
                }
            }
        }
        buffer
    }

    /// TAB completion: complete the last token against built-in command names.
    ///
    /// Single match: erase the partial token and insert the full name.
    /// Multiple matches: print candidates on a new line, then reprint the buffer.
    fn handle_tab(buffer: &mut Vec<u8>) {
        // Extract the last whitespace-delimited token.
        let line = core::str::from_utf8(buffer).unwrap_or("");
        let token = line.split_whitespace().last().unwrap_or("");
        let token_bytes = token.len();

        // Match against built-in names.
        let matches: alloc::vec::Vec<&str> = crate::executor::BUILTINS
            .iter()
            .filter(|b| b.starts_with(token))
            .copied()
            .collect();

        match matches.len() {
            0 => { /* bell or no-op */ }
            1 => {
                // Erase the partial token, insert the completed name + space.
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
                // Print all matches on a new line, then reprint the current buffer.
                ostd::io::print("\n");
                for (i, m) in matches.iter().enumerate() {
                    if i > 0 { ostd::io::print("  "); }
                    ostd::io::print(m);
                }
                ostd::io::print("\n");
                // Reprint the buffer so the user can continue editing.
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
