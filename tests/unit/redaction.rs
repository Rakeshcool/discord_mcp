//! Redaction of tokens, authorization headers and registered literals.

use std::io::Write;
use std::sync::{Arc, Mutex};

use discord_mcp::security::redaction::{REDACTED, RedactingWriter, Redactor};

const TOKEN: &str = "MTAwMDAwMDAwMDAwMDAwMDAw.Ga4dEf.abcdefghijklmnopqrstuvwxyz0123456789AB";

#[test]
fn bot_tokens_are_scrubbed_whatever_their_value() {
    let redactor = Redactor::new();
    let line = format!("request headers: Authorization: Bot {TOKEN}");

    let redacted = redactor.redact(&line);
    assert!(!redacted.contains(TOKEN), "token survived: {redacted}");
    assert!(redacted.contains(REDACTED), "no marker: {redacted}");
}

#[test]
fn bearer_tokens_are_scrubbed() {
    let redactor = Redactor::new();
    let redacted = redactor.redact(&format!("Authorization: Bearer {TOKEN}"));

    assert!(!redacted.contains(TOKEN), "token survived: {redacted}");
}

#[test]
fn credential_key_values_are_scrubbed() {
    let redactor = Redactor::new();

    let header = redactor.redact(&format!("authorization: {TOKEN} status=200"));
    assert!(!header.contains(TOKEN), "token survived: {header}");
    assert!(header.contains("status=200"), "lost context: {header}");

    let query = redactor.redact(&format!("token={TOKEN}&scope=bot"));
    assert!(!query.contains(TOKEN), "token survived: {query}");
}

#[test]
fn ordinary_diagnostics_are_untouched() {
    let redactor = Redactor::new();
    let line = "request failed after 2 attempts; rate limit remaining 4, reset in 1500ms";

    assert_eq!(redactor.redact(line), line);
}

#[test]
fn words_containing_bot_are_not_treated_as_credentials() {
    let redactor = Redactor::new();
    let line = "Robot nickname changed for member 123456789012345678";

    assert_eq!(redactor.redact(line), line);
}

#[test]
fn registered_literals_are_removed() {
    let redactor = Redactor::with_literals([TOKEN.to_string()]);
    let redacted = redactor.redact(&format!("token value leaked here: {TOKEN}!"));

    assert!(!redacted.contains(TOKEN), "literal survived: {redacted}");
    assert!(redactor.contains_literal(&format!("x{TOKEN}x")));
}

#[test]
fn short_literals_are_ignored() {
    let redactor = Redactor::with_literals(["abc"]);
    assert!(!redactor.contains_literal("abc"));
    assert_eq!(redactor.redact("abc"), "abc");
}

#[test]
fn the_log_writer_redacts_every_line_it_forwards() {
    let sink = SharedSink::default();
    {
        let mut writer = RedactingWriter::new(sink.clone(), Redactor::new());
        writer
            .write_all(format!("line one: Authorization: Bot {TOKEN}\n").as_bytes())
            .expect("write");
        writer
            .write_all(b"line two: nothing sensitive\n")
            .expect("write");
        writer.flush().expect("flush");
    }

    let text = sink.contents();
    assert!(!text.contains(TOKEN), "writer leaked the token: {text}");
    assert!(text.contains("line two"), "writer dropped a line: {text}");
    assert_eq!(text.lines().count(), 2);
}

#[test]
fn the_log_writer_flushes_a_trailing_partial_line_on_drop() {
    let sink = SharedSink::default();
    {
        let mut writer = RedactingWriter::new(sink.clone(), Redactor::new());
        writer
            .write_all(format!("no newline: Bot {TOKEN}").as_bytes())
            .expect("write");
    }

    let text = sink.contents();
    assert!(!text.contains(TOKEN), "writer leaked the token: {text}");
    assert!(text.contains("no newline"), "lost text: {text}");
}

/// A shared byte sink, so the writer can be inspected after it is dropped.
#[derive(Clone, Default)]
struct SharedSink {
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl SharedSink {
    fn contents(&self) -> String {
        let guard = self.bytes.lock().expect("sink lock");
        String::from_utf8_lossy(&guard).into_owned()
    }
}

impl Write for SharedSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.bytes.lock().expect("sink lock").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
