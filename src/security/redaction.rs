//! Secret redaction for logs, errors and tool output.
//!
//! Two independent mechanisms, so a mistake in one does not leak a token:
//!
//! 1. **Structural scrubbing** — anything shaped like `Bot <token>`,
//!    `Bearer <token>` or a `token=…` / `authorization: …` pair is replaced
//!    whatever its value. This needs no knowledge of the secret.
//! 2. **Literal scrubbing** — optional registered values, used when a caller
//!    knows a secret and wants belt-and-braces removal.
//!
//! [`RedactedStderr`] applies both to every tracing event, so diagnostics can
//! never print a token even if a `#[derive(Debug)]` type later gains a
//! sensitive field.

use std::fmt;
use std::io::{self, Write};

/// Replacement text for removed secrets.
pub const REDACTED: &str = "[REDACTED]";

/// Literals shorter than this are not registered: scrubbing them would mangle
/// unrelated text without protecting anything.
const MIN_LITERAL_LEN: usize = 8;

/// Prefixes that introduce a credential value.
const SECRET_PREFIXES: [&str; 2] = ["Bot ", "Bearer "];

/// Keys whose values are credentials, matched case-insensitively.
const SECRET_KEYS: [&str; 3] = ["authorization", "x-api-key", "token"];

/// Characters that may appear inside a token.
fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '~' | '+' | '/' | '=' | ':')
}

/// The redactor. Cheap to clone; cloned into each log writer.
#[derive(Clone, Default)]
pub struct Redactor {
    literals: Vec<String>,
}

impl Redactor {
    /// A redactor with structural scrubbing only.
    pub fn new() -> Self {
        Self::default()
    }

    /// A redactor that also removes the given literal values.
    ///
    /// Values shorter than 8 characters are ignored.
    pub fn with_literals<I, S>(literals: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut literals: Vec<String> = literals
            .into_iter()
            .map(Into::into)
            .filter(|literal| literal.len() >= MIN_LITERAL_LEN)
            .collect();
        // Longest first, so a secret that contains another is fully removed.
        literals.sort_by_key(|literal| std::cmp::Reverse(literal.len()));
        literals.dedup();
        Self { literals }
    }

    /// Whether a registered literal occurs in `input`.
    pub fn contains_literal(&self, input: &str) -> bool {
        self.literals.iter().any(|literal| input.contains(literal))
    }

    /// Remove secrets from `input`.
    pub fn redact(&self, input: &str) -> String {
        let mut output = input.to_string();
        for literal in &self.literals {
            if output.contains(literal.as_str()) {
                output = output.replace(literal.as_str(), REDACTED);
            }
        }
        let output = scrub_prefixed_secrets(&output);
        scrub_key_values(&output)
    }
}

impl fmt::Debug for Redactor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Redactor")
            .field("literals", &self.literals.len())
            .finish()
    }
}

/// Replace the value after `Bot ` / `Bearer ` when it looks like a credential.
fn scrub_prefixed_secrets(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;

    loop {
        let mut earliest: Option<(usize, usize)> = None;
        for prefix in SECRET_PREFIXES {
            if let Some(index) = find_prefixed_secret(rest, prefix) {
                let better = earliest.is_none_or(|(best_index, _)| index < best_index);
                if better {
                    earliest = Some((index, prefix.len()));
                }
            }
        }

        let Some((index, prefix_len)) = earliest else {
            output.push_str(rest);
            break;
        };

        output.push_str(&rest[..index + prefix_len]);
        let after_prefix = &rest[index + prefix_len..];
        let value_end = after_prefix
            .find(|c: char| !is_token_char(c))
            .unwrap_or(after_prefix.len());
        if value_end >= 16 {
            output.push_str(REDACTED);
        } else {
            output.push_str(&after_prefix[..value_end]);
        }
        rest = &after_prefix[value_end..];
    }

    output
}

/// Replace values that follow a credential key, per line.
fn scrub_key_values(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for line in input.split_inclusive('\n') {
        output.push_str(&scrub_line(line));
    }
    output
}

fn scrub_line(line: &str) -> String {
    let lowered = line.to_ascii_lowercase();
    for key in SECRET_KEYS {
        let mut search_from = 0;
        while let Some(offset) = lowered[search_from..].find(key) {
            let key_end = search_from + offset + key.len();
            let after_key = &lowered[key_end..];
            let Some(separator) = after_key.find([':', '=']) else {
                break;
            };
            if !after_key[..separator].trim().is_empty() {
                search_from = key_end;
                continue;
            }

            let value_after_separator = key_end + separator + 1;
            let value_and_rest = &line[value_after_separator..];
            let leading_space = value_and_rest.len() - value_and_rest.trim_start().len();
            let value_start = value_after_separator + leading_space;
            let value = &line[value_start..];
            let value_len = value.find(char::is_whitespace).unwrap_or(value.len());

            let mut redacted = String::with_capacity(line.len());
            redacted.push_str(&line[..value_start]);
            redacted.push_str(REDACTED);
            redacted.push_str(&value[value_len..]);
            return redacted;
        }
    }
    line.to_string()
}

/// Find `prefix` where it starts a word, so `Robot` does not match `Bot `.
fn find_prefixed_secret(haystack: &str, prefix: &str) -> Option<usize> {
    let lowered = haystack.to_ascii_lowercase();
    let needle = prefix.to_ascii_lowercase();
    let mut search_from = 0;
    while let Some(offset) = lowered[search_from..].find(&needle) {
        let index = search_from + offset;
        let at_word_start = haystack[..index]
            .chars()
            .next_back()
            .is_none_or(|previous| !previous.is_ascii_alphanumeric());
        if at_word_start {
            return Some(index);
        }
        search_from = index + needle.len();
    }
    None
}

/// A writer that redacts each complete line before forwarding it.
pub struct RedactingWriter<W: Write> {
    inner: W,
    redactor: Redactor,
    pending: Vec<u8>,
}

impl<W: Write> RedactingWriter<W> {
    /// Wrap `inner` so every line written through it is redacted.
    pub fn new(inner: W, redactor: Redactor) -> Self {
        Self {
            inner,
            redactor,
            pending: Vec::new(),
        }
    }
}

impl<W: Write> Write for RedactingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(buf);
        while let Some(position) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=position).collect();
            let text = String::from_utf8_lossy(&line);
            self.inner
                .write_all(self.redactor.redact(&text).as_bytes())?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.pending.is_empty() {
            let text = String::from_utf8_lossy(&self.pending).into_owned();
            self.pending.clear();
            self.inner
                .write_all(self.redactor.redact(&text).as_bytes())?;
        }
        self.inner.flush()
    }
}

impl<W: Write> fmt::Debug for RedactingWriter<W> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedactingWriter")
            .field("pending_bytes", &self.pending.len())
            .finish_non_exhaustive()
    }
}

impl<W: Write> Drop for RedactingWriter<W> {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

/// `tracing` writer that sends redacted lines to stderr.
///
/// stdout stays untouched: it belongs to the MCP transport.
#[derive(Clone, Debug)]
pub struct RedactedStderr {
    redactor: Redactor,
}

impl RedactedStderr {
    /// Build a stderr writer that redacts with `redactor`.
    pub fn new(redactor: Redactor) -> Self {
        Self { redactor }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for RedactedStderr {
    type Writer = RedactingWriter<io::Stderr>;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter::new(io::stderr(), self.redactor.clone())
    }
}
