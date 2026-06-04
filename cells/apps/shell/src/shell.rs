use crate::aliases::Aliases;
use crate::async_utils::AsyncStdin;
use crate::config_client::ConfigClient;
use crate::executor;
use crate::history::History;
use crate::jobs::Jobs;
use crate::parser;
use api::config::ViConfig;
use ostd::prelude::*;

use alloc::collections::VecDeque;
use alloc::string::String;

pub struct ViShell<'a> {
    prompt: &'a str,
    config: ConfigClient,
    /// Legacy inline history (kept for AsyncStdin arrow-key compat).
    history: VecDeque<String>,
    /// New persistent history module.
    hist: History,
    jobs: Jobs,
    aliases: Aliases,
}

impl<'a> ViShell<'a> {
    pub fn new() -> Self {
        Self {
            prompt: "ViOS > ",
            config: ConfigClient::new(2),
            history: VecDeque::with_capacity(32),
            hist: History::new(),
            jobs: Jobs::new(),
            aliases: Aliases::new(),
        }
    }

    pub async fn run(&mut self) {
        let stdin = AsyncStdin;
        loop {
            // Show custom prompt if PATH set? Or USER?
            // For now static prompt.
            ostd::io::print(self.prompt);

            let buffer = stdin.read_line(128, &mut self.history).await;
            let len = buffer.len();

            if len > 0 {
                if let Ok(line) = core::str::from_utf8(&buffer) {
                    // Add to history if not empty and not repeat of last
                    let trim_line = line.trim();
                    // Skip comment lines (# prefix) without executing or adding to history.
                    if trim_line.starts_with('#') { continue; }
                    if !trim_line.is_empty() {
                         if self.history.back().map(|s| s.as_str()) != Some(trim_line) {
                             if self.history.len() >= 32 {
                                 self.history.pop_front();
                             }
                             self.history.push_back(String::from(trim_line));
                         }
                    }

                    let _ = self.dispatch(line).await;
                    // Check for `exit N` — built-in sets this flag.
                    if crate::executor::take_exit_request().is_some() {
                        return;
                    }
                }
            }
        }
    }

    /// Dispatch one shell line through the parser + executor.
    ///
    /// Alias expansion is applied before parsing.  Special built-ins that need
    /// direct shell state (`alias`, `unalias`, `export`, `echo`) are handled
    /// here before handing off to the executor.
    pub async fn dispatch(&mut self, line: &str) -> ViResult<()> {
        let trimmed = line.trim();
        if trimmed.is_empty() { return Ok(()); }

        // ── Alias expansion ───────────────────────────────────────────────
        let expanded_storage;
        let effective = if let Some(exp) = self.aliases.expand(trimmed) {
            expanded_storage = exp;
            expanded_storage.as_str()
        } else {
            trimmed
        };

        // ── Shell-state built-ins (need &mut self) ────────────────────────
        let mut parts = effective.split_whitespace();
        let first = parts.next().unwrap_or("");

        match first {
            "alias" => {
                if let Some(arg) = parts.next() {
                    if let Some((k, v)) = arg.split_once('=') {
                        self.aliases.set(k, v.trim_matches('\'').trim_matches('"'));
                    }
                } else {
                    for (k, v) in self.aliases.list() {
                        ostd::io::print(k);
                        ostd::io::print("='");
                        ostd::io::print(v);
                        ostd::io::println("'");
                    }
                }
                return Ok(());
            }
            "unalias" => {
                if let Some(name) = parts.next() {
                    self.aliases.remove(name);
                }
                return Ok(());
            }
            "export" => {
                if let Some(arg) = parts.next() {
                    if let Some((k, v)) = arg.split_once('=') {
                        let mut client = ConfigClient::new(2);
                        let _ = client.set(k, v);
                    }
                }
                return Ok(());
            }
            _ => {}
        }

        // ── Parse + execute ───────────────────────────────────────────────
        let ast = parser::parse(effective);
        executor::execute(&ast, &mut self.jobs);
        self.hist.push(effective);
        self.jobs.reap_done();
        Ok(())
    }
}
