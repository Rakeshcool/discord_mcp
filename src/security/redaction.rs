//! Secret redaction for logs and diagnostics.
//!
//! Redaction is **structural**, not value-based: anything shaped like
//! `Bot <token>`, `Bearer <token>`, or the value of an `authorization`,
//! `x-api-key` or `token` key is removed whatever it contains. Nothing has to be
//! registered with the redactor, so a token it never saw cannot reach a log
//! line, and no second copy of a secret is kept in memory.
//!
//! [`RedactedStderr`] applies this to every tracing event, so a diagnostic can
//! never print a credential even if a `#[derive(Debug)]` type later gains a
//! sensitive field.

use std::fmt;
use std::io::{self, Write};

/// Replacement text for removed credentials.
pub const REDACTED: &str = "[REDACTED]";

/// Prefixes that introduce a credential value, lowercase for matching.
const CREDENTIAL_PREFIXES: [&str; 2] = ["bot ", "bearer "];

/// Keys whose values are credentials, lowercase for matching.
const CREDENTIAL_KEYS: [&str; 3] = ["authorization", "x-api-key", "token"];

/// Shortest run of characters treated as a credential. Shorter runs are words.
const MIN_CREDENTIAL_LEN: usize = 16;

/// Remove credential-shaped values from `input`.
pub fn redact(input: &str) -> String {
    scrub_key_values(&scrub_prefixed_credentials(input))
}

/// Characters that may appear inside a token.
fn is_credential_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '~' | '+' | '/' | '=')
}

/// Replace values introduced by `Bot ` / `Bearer `.
fn scrub_prefixed_credentials(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;

    while let Some((index, prefix_len)) = find_credential_prefix(rest) {
        output.push_str(&rest[..index + prefix_len]);
        let after_prefix = &rest[index + prefix_len..];
        let value_len = after_prefix
            .find(|c: char| !is_credential_char(c))
            .unwrap_or(after_prefix.len());
        if value_len >= MIN_CREDENTIAL_LEN {
            output.push_str(REDACTED);
        } else {
            output.push_str(&after_prefix[..value_len]);
        }
        rest = &after_prefix[value_len..];
    }

    output.push_str(rest);
    output
}

/// The earliest `Bot `/`Bearer ` that starts a word, so `Robot` does not match.
fn find_credential_prefix(text: &str) -> Option<(usize, usize)> {
    let lowered = text.to_ascii_lowercase();
    CREDENTIAL_PREFIXES
        .iter()
        .filter_map(|prefix| {
            let index = lowered
                .match_indices(prefix)
                .map(|(index, _)| index)
                .find(|index| {
                    text[..*index]
                        .chars()
                        .next_back()
                        .is_none_or(|previous| !previous.is_ascii_alphanumeric())
                })?;
            Some((index, prefix.len()))
        })
        .min_by_key(|(index, _)| *index)
}

/// Replace credential values after a key, one line at a time.
fn scrub_key_values(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for line in input.split_inclusive('\n') {
        output.push_str(&scrub_line(line));
    }
    output
}

/// `authorization: <secret>` / `token=<secret>`, keeping trailing context.
fn scrub_line(line: &str) -> String {
    let lowered = line.to_ascii_lowercase();
    for key in CREDENTIAL_KEYS {
        for (key_start, _) in lowered.match_indices(key) {
            let after_key = &lowered[key_start + key.len()..];
            // Only a separator immediately after the key starts a value; this
            // keeps prose such as "token bucket reset" untouched.
            let Some(separator) = after_key.find([':', '=']) else {
                break;
            };
            if !after_key[..separator].trim().is_empty() {
                continue;
            }
            return redact_value_after(line, key_start + key.len() + separator + 1);
        }
    }
    line.to_string()
}

/// Redact the whitespace-delimited value that follows `after_separator`.
fn redact_value_after(line: &str, after_separator: usize) -> String {
    let value_and_rest = &line[after_separator..];
    let start = after_separator + (value_and_rest.len() - value_and_rest.trim_start().len());
    let value = &line[start..];
    let value_len = value.find(char::is_whitespace).unwrap_or(value.len());

    let mut output = String::with_capacity(line.len());
    output.push_str(&line[..start]);
    output.push_str(REDACTED);
    output.push_str(&value[value_len..]);
    output
}

/// A writer that redacts each complete line before forwarding it.
pub struct RedactingWriter<W: Write> {
    inner: W,
    pending: Vec<u8>,
}

impl<W: Write> RedactingWriter<W> {
    /// Wrap `inner` so every line written through it is redacted.
    pub fn new(inner: W) -> Self {
        Self {
            inner,
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
            self.inner.write_all(redact(&text).as_bytes())?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.pending.is_empty() {
            let text = String::from_utf8_lossy(&self.pending).into_owned();
            self.pending.clear();
            self.inner.write_all(redact(&text).as_bytes())?;
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
#[derive(Clone, Copy, Debug, Default)]
pub struct RedactedStderr;

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for RedactedStderr {
    type Writer = RedactingWriter<io::Stderr>;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter::new(io::stderr())
    }
}
