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
    /// `cmd1 && cmd2` — run cmd2 only if cmd1 exits 0 (success).
    And(alloc::boxed::Box<Ast>, alloc::boxed::Box<Ast>),
    /// `cmd1 || cmd2` — run cmd2 only if cmd1 exits non-zero (failure).
    Or(alloc::boxed::Box<Ast>, alloc::boxed::Box<Ast>),
    /// `while COND; do BODY; done` — loop while COND exits 0.
    While {
        cond: alloc::boxed::Box<Ast>,
        body: alloc::boxed::Box<Ast>,
    },
    /// `for VAR in word1 word2 …; do BODY; done` — iterate over a word list.
    ///
    /// Sets `$VAR` to each word in order, runs BODY, then advances. `$VAR`
    /// expansion in BODY uses the same static var store as `VAR=value`.
    For {
        var:   alloc::string::String,
        words: alloc::vec::Vec<alloc::string::String>,
        body:  alloc::boxed::Box<Ast>,
    },
    /// `if COND; then BODY; fi` — conditional execution.
    ///
    /// `cond` exit-code 0 → run `then_b`; non-zero → run `else_b` if present.
    If {
        cond:   alloc::boxed::Box<Ast>,
        then_b: alloc::boxed::Box<Ast>,
        else_b: Option<alloc::boxed::Box<Ast>>,
    },
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
    Ampersand,      // &   (background marker — single &)
    And,            // &&  (short-circuit AND)
    Or,             // ||  (short-circuit OR)
    Semicolon,      // ;
    // ── Conditional keywords ─────────────────────────────────────────────────
    // These variants are NEVER emitted by the tokenizer — `if`/`then`/`else`/`fi`
    // always remain as Word tokens.  parse_if_stmt detects them by string
    // comparison so they never silently disappear from external command arguments
    // (e.g. `lua -e "if x then ... end"` must reach Lua intact).
    If,   // reserved — kept for exhaustive match arms in parse_cmd
    Then, // reserved
    Else, // reserved
    Fi,   // reserved
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
            '|' => {
                flush!();
                if chars.peek() == Some(&'|') { chars.next(); tokens.push(Tok::Or); }
                else { tokens.push(Tok::Pipe); }
            }
            '&' => {
                flush!();
                if chars.peek() == Some(&'&') { chars.next(); tokens.push(Tok::And); }
                else { tokens.push(Tok::Ampersand); }
            }
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
    // All tokens remain as their natural type — no keyword conversion.
    // The if-statement parser detects `if`/`then`/`else`/`fi` by string
    // comparison on Word tokens so they are never eaten from command arguments.
    tokens
}

// ─── Parser ───────────────────────────────────────────────────────────────────

/// Parse a shell line into an `Ast`.
///
/// Returns `Ast::Empty` for blank input.
pub fn parse(line: &str) -> Ast {
    let tokens = tokenize(line.trim());
    if tokens.is_empty() { return Ast::Empty; }

    // `if...then...fi` must be parsed BEFORE semicolon splitting, because the
    // semicolons inside an if-statement are structural (not sequence separators).
    // Keywords remain as Word tokens (not converted) so they survive in external
    // command argument strings (e.g. `lua -e "if x then ... end"`).
    if tokens.first() == Some(&Tok::Word("if".into())) {
        return parse_if_stmt(&tokens);
    }
    if tokens.first() == Some(&Tok::Word("while".into())) {
        return parse_while_stmt(&tokens);
    }
    if tokens.first() == Some(&Tok::Word("for".into())) {
        return parse_for_stmt(&tokens);
    }

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

/// Parse a token sub-slice that may contain `;`-separated commands.
///
/// Equivalent to the main `parse()` body but operating on a pre-tokenized
/// slice — used by `parse_if_stmt` to parse condition and body sections.
fn parse_tokens(tokens: &[Tok]) -> Ast {
    // Strip leading/trailing semicolons that linger from the structural split.
    let start = tokens.iter().position(|t| t != &Tok::Semicolon).unwrap_or(tokens.len());
    let end   = tokens.iter().rposition(|t| t != &Tok::Semicolon).map(|i| i + 1).unwrap_or(0);
    let tokens = &tokens[start..end];
    if tokens.is_empty() { return Ast::Empty; }
    let segments: Vec<&[Tok]> = split_on(tokens, |t| t == &Tok::Semicolon);
    if segments.len() > 1 {
        let seq: Vec<Ast> = segments.iter().map(|seg| parse_pipeline(seg)).collect();
        return Ast::Sequence(seq);
    }
    parse_pipeline(tokens)
}

/// Parse `if COND; then BODY; fi` or `if COND; then BODY; else BODY; fi`.
///
/// Handles any number of semicolons around the keywords — the structure is
/// determined by the `Then`, `Else`, and `Fi` token positions.
/// Helper: returns true when a token is the keyword word `w`.
fn is_kw(tok: &Tok, w: &str) -> bool {
    tok == &Tok::Word(w.into())
}

/// Parse `while COND; do BODY; done`.
///
/// Keywords stay as `Word` tokens (no Tok variants) so `while`/`do`/`done`
/// survive intact when used as external command arguments.  Malformed input
/// (missing `do` or `done`) calls `parse_tokens` — NOT `parse()` — to avoid
/// re-dispatching on the leading `while` and recursing infinitely.
/// Parse `for VAR in word1 word2 …; do BODY; done`.
///
/// Keywords stay as `Word` tokens (same Phase N/O rule) so `for`/`in`/`do`/`done`
/// survive as external command arguments.  Malformed input falls back to
/// `parse_tokens` (not `parse()`) to prevent infinite recursion.
fn parse_for_stmt(tokens: &[Tok]) -> Ast {
    // tokens[1] = variable name; tokens[2] should be Word("in").
    let var = match tokens.get(1) {
        Some(Tok::Word(w)) if w != "in" => w.clone(),
        _ => return parse_tokens(tokens),
    };
    let in_pos = match tokens.iter().position(|t| is_kw(t, "in")) {
        Some(p) => p,
        None => return parse_tokens(tokens),
    };
    let do_pos   = tokens.iter().position(|t| is_kw(t, "do"));
    let done_pos = tokens.iter().rposition(|t| is_kw(t, "done"));
    let (dp, np) = match (do_pos, done_pos) {
        (Some(d), Some(n)) if n > d => (d, n),
        _ => return parse_tokens(tokens),
    };
    // Word list: tokens between `in` and `do`, stripping Semicolons.
    let words: alloc::vec::Vec<alloc::string::String> = tokens[in_pos + 1..dp]
        .iter()
        .filter_map(|t| if let Tok::Word(w) = t { Some(w.clone()) } else { None })
        .collect();
    let body = parse_tokens(&tokens[dp + 1..np]);
    Ast::For {
        var,
        words,
        body: alloc::boxed::Box::new(body),
    }
}

fn parse_while_stmt(tokens: &[Tok]) -> Ast {
    let do_pos   = tokens.iter().position(|t| is_kw(t, "do"));
    let done_pos = tokens.iter().rposition(|t| is_kw(t, "done"));
    let (dp, np) = match (do_pos, done_pos) {
        (Some(d), Some(n)) if n > d => (d, n),
        _ => return parse_tokens(tokens),   // malformed: fall back without infinite recursion
    };
    let cond = parse_tokens(&tokens[1..dp]);
    let body = parse_tokens(&tokens[dp + 1..np]);
    Ast::While {
        cond: alloc::boxed::Box::new(cond),
        body: alloc::boxed::Box::new(body),
    }
}

fn parse_if_stmt(tokens: &[Tok]) -> Ast {
    // Locate structural keywords after the leading `if` Word.
    // Keywords are plain Word tokens — never converted — so they survive intact
    // in external command argument strings.
    let then_pos = tokens.iter().position(|t| is_kw(t, "then")).unwrap_or(tokens.len());
    let else_pos = tokens.iter().position(|t| is_kw(t, "else"));
    let fi_pos   = tokens.iter().rposition(|t| is_kw(t, "fi")).unwrap_or(tokens.len());

    // Condition: tokens[1..then_pos]   (skip leading `If`)
    let cond_slice = &tokens[1..then_pos];
    let cond = parse_tokens(cond_slice);

    // Then body: tokens[then_pos+1..else_or_fi]
    let then_end = else_pos.unwrap_or(fi_pos);
    let then_slice = &tokens[then_pos + 1..then_end];
    let then_b = parse_tokens(then_slice);

    // Else body (optional): tokens[else_pos+1..fi_pos]
    let else_b = else_pos.map(|ep| {
        let slice = &tokens[ep + 1..fi_pos];
        alloc::boxed::Box::new(parse_tokens(slice))
    });

    Ast::If {
        cond:   alloc::boxed::Box::new(cond),
        then_b: alloc::boxed::Box::new(then_b),
        else_b,
    }
}

fn parse_pipeline(tokens: &[Tok]) -> Ast {
    // `&&` / `||` have lower precedence than pipelines — check first.
    // Split on the FIRST occurrence; right side is parsed recursively so
    // `A && B && C` builds `And(A, And(B, C))` with left-to-right evaluation.
    if let Some(pos) = tokens.iter().position(|t| t == &Tok::And || t == &Tok::Or) {
        let left  = parse_pipeline(&tokens[..pos]);
        let right = parse_pipeline(&tokens[pos + 1..]);
        return match &tokens[pos] {
            Tok::And => Ast::And(alloc::boxed::Box::new(left), alloc::boxed::Box::new(right)),
            Tok::Or  => Ast::Or(alloc::boxed::Box::new(left),  alloc::boxed::Box::new(right)),
            _ => unreachable!(),
        };
    }

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
