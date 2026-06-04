//! Shell command parser — tokenizes a line and builds an `Ast`.
//!
//! Supported syntax (v1.0):
//!   - Simple command: `ls /bin`
//!   - Pipeline: `cat /etc/hosts | grep 127`
//!   - Output redirect: `echo hello > /tmp/a.txt`
//!   - Input redirect: `cat < /tmp/a.txt`
//!   - Append redirect: `echo hi >> /tmp/log.txt`
//!   - Background: `sleep 10 &`
//!   - Sequence: `echo a ; echo b`
//!
//! Intentionally simple: no subshells, no quoting beyond `"..."`, no globs.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

/// One redirect target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redirect {
    /// `> path`
    StdoutTo(String),
    /// `>> path`
    StdoutAppend(String),
    /// `< path`
    StdinFrom(String),
    /// `2> path`
    StderrTo(String),
}

/// A single command with its arguments and any redirects.
#[derive(Debug, Clone)]
pub struct Cmd {
    /// `argv[0]` and arguments.
    pub argv: Vec<String>,
    /// Redirects attached to this command.
    pub redirects: Vec<Redirect>,
}

impl Cmd {
    fn new() -> Self { Cmd { argv: Vec::new(), redirects: Vec::new() } }

    /// True if the command has no name (empty line or whitespace-only).
    pub fn is_empty(&self) -> bool { self.argv.is_empty() }
}

/// Top-level abstract syntax tree for one shell line.
#[derive(Debug, Clone)]
pub enum Ast {
    /// Empty input.
    Empty,
    /// A single simple command.
    Simple(Cmd),
    /// `cmd1 | cmd2 | …` — pipeline of commands.
    Pipeline(Vec<Cmd>),
    /// `cmd &` — run in background.
    Background(Cmd),
    /// `cmd1 ; cmd2` — sequential execution.
    Sequence(Vec<Ast>),
}

// ─── Tokenizer ────────────────────────────────────────────────────────────────

/// Raw token before AST construction.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Word(String),
    Pipe,           // |
    RedirectOut,    // >
    RedirectAppend, // >>
    RedirectIn,     // <
    RedirectErr,    // 2>
    Ampersand,      // &
    Semicolon,      // ;
}

/// Tokenize a shell input line.
///
/// Handles:
/// - Whitespace separation
/// - Simple `"..."` double-quoted strings (no escape sequences)
/// - Single-character operators: `|`, `<`, `>`, `&`, `;`
/// - Two-character operators: `>>`, `2>`
fn tokenize(line: &str) -> Vec<Tok> {
    let mut tokens = Vec::new();
    let mut chars = line.chars().peekable();
    let mut current = String::new();

    macro_rules! flush {
        () => {
            if !current.is_empty() {
                tokens.push(Tok::Word(current.clone()));
                current.clear();
            }
        };
    }

    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => { flush!(); }
            '"' => {
                // Consume until closing '"'.
                loop {
                    match chars.next() {
                        Some('"') | None => break,
                        Some(ch) => current.push(ch),
                    }
                }
            }
            '|' => { flush!(); tokens.push(Tok::Pipe); }
            '&' => { flush!(); tokens.push(Tok::Ampersand); }
            ';' => { flush!(); tokens.push(Tok::Semicolon); }
            '<' => { flush!(); tokens.push(Tok::RedirectIn); }
            '>' => {
                flush!();
                if chars.peek() == Some(&'>') {
                    chars.next();
                    tokens.push(Tok::RedirectAppend);
                } else {
                    tokens.push(Tok::RedirectOut);
                }
            }
            '2' if chars.peek() == Some(&'>') => {
                // "2>" — only if current buffer is empty (i.e. not part of a word).
                if current.is_empty() {
                    chars.next();
                    tokens.push(Tok::RedirectErr);
                } else {
                    current.push(c);
                }
            }
            other => { current.push(other); }
        }
    }
    flush!();
    tokens
}

// ─── Parser ───────────────────────────────────────────────────────────────────

/// Parse a shell line into an `Ast`.
///
/// Returns `Ast::Empty` for blank input.
pub fn parse(line: &str) -> Ast {
    let tokens = tokenize(line.trim());
    if tokens.is_empty() { return Ast::Empty; }

    // Split on `;` into sub-sequences first.
    let segments: Vec<&[Tok]> = split_on(&tokens, |t| t == &Tok::Semicolon);
    if segments.len() > 1 {
        let seq: Vec<Ast> = segments.iter()
            .map(|seg| parse_pipeline(seg))
            .collect();
        return Ast::Sequence(seq);
    }

    parse_pipeline(&tokens)
}

fn parse_pipeline(tokens: &[Tok]) -> Ast {
    let pipe_segs: Vec<&[Tok]> = split_on(tokens, |t| t == &Tok::Pipe);

    let cmds: Vec<Cmd> = pipe_segs.iter()
        .filter_map(|seg| {
            // Ignore the per-segment `bg` flag here — the trailing `&` check on
            // `tokens.last()` below is the authoritative background detector.
            // Filtering out `bg=true` segments caused single-command background
            // jobs (`httpd 9091 /path &`) to be parsed as Ast::Empty.
            let (cmd, _bg) = parse_cmd(seg);
            Some(cmd)
        })
        .filter(|c| !c.is_empty())
        .collect();

    // Check for trailing `&` (background marker).
    let background = tokens.last() == Some(&Tok::Ampersand);

    if background && cmds.len() == 1 {
        return Ast::Background(cmds.into_iter().next().unwrap_or_else(Cmd::new));
    }
    match cmds.len() {
        0 => Ast::Empty,
        1 => Ast::Simple(cmds.into_iter().next().unwrap_or_else(Cmd::new)),
        _ => Ast::Pipeline(cmds),
    }
}

/// Parse one command segment (no `|`, `;`, or `&` except trailing `&`).
/// Returns (Cmd, is_background).
fn parse_cmd(tokens: &[Tok]) -> (Cmd, bool) {
    let mut cmd = Cmd::new();
    let mut background = false;
    let mut iter = tokens.iter().peekable();

    while let Some(tok) = iter.next() {
        match tok {
            Tok::Word(w) => cmd.argv.push(w.clone()),
            Tok::Ampersand => background = true,
            Tok::RedirectOut => {
                if let Some(Tok::Word(path)) = iter.next() {
                    cmd.redirects.push(Redirect::StdoutTo(path.clone()));
                }
            }
            Tok::RedirectAppend => {
                if let Some(Tok::Word(path)) = iter.next() {
                    cmd.redirects.push(Redirect::StdoutAppend(path.clone()));
                }
            }
            Tok::RedirectIn => {
                if let Some(Tok::Word(path)) = iter.next() {
                    cmd.redirects.push(Redirect::StdinFrom(path.clone()));
                }
            }
            Tok::RedirectErr => {
                if let Some(Tok::Word(path)) = iter.next() {
                    cmd.redirects.push(Redirect::StderrTo(path.clone()));
                }
            }
            _ => {}
        }
    }
    (cmd, background)
}

/// Split a token slice on positions where `pred` returns true.
fn split_on<'a, F>(tokens: &'a [Tok], pred: F) -> Vec<&'a [Tok]>
where F: Fn(&Tok) -> bool
{
    let mut result = Vec::new();
    let mut start = 0;
    for (i, tok) in tokens.iter().enumerate() {
        if pred(tok) {
            result.push(&tokens[start..i]);
            start = i + 1;
        }
    }
    result.push(&tokens[start..]);
    result
}

// ─── Tests (host-runnable) ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty() {
        assert!(matches!(parse(""), Ast::Empty));
        assert!(matches!(parse("   "), Ast::Empty));
    }

    #[test]
    fn parse_simple() {
        if let Ast::Simple(cmd) = parse("ls /bin") {
            assert_eq!(cmd.argv, &["ls", "/bin"]);
        } else { panic!("expected Simple"); }
    }

    #[test]
    fn parse_pipeline() {
        if let Ast::Pipeline(cmds) = parse("cat /etc/hosts | grep 127") {
            assert_eq!(cmds.len(), 2);
            assert_eq!(cmds[0].argv[0], "cat");
            assert_eq!(cmds[1].argv[0], "grep");
        } else { panic!("expected Pipeline"); }
    }

    #[test]
    fn parse_redirect_out() {
        if let Ast::Simple(cmd) = parse("echo hi > /tmp/a.txt") {
            assert_eq!(cmd.redirects, &[Redirect::StdoutTo(String::from("/tmp/a.txt"))]);
        } else { panic!("expected Simple with redirect"); }
    }

    #[test]
    fn parse_redirect_append() {
        if let Ast::Simple(cmd) = parse("echo hi >> /tmp/log") {
            assert!(matches!(&cmd.redirects[0], Redirect::StdoutAppend(_)));
        } else { panic!("expected Simple with append redirect"); }
    }

    #[test]
    fn parse_background() {
        assert!(matches!(parse("sleep 10 &"), Ast::Background(_)));
    }

    #[test]
    fn parse_sequence() {
        assert!(matches!(parse("echo a ; echo b"), Ast::Sequence(_)));
    }
}
