//! Bot token shape validation and redacted `Debug`.

use discord_mcp::security::secrets::BotToken;

const VARIABLE: &str = "DISCORD_MCP_TEST_TOKEN";
const VALID: &str = "MTAwMDAwMDAwMDAwMDAwMDAw.Ga4dEf.abcdefghijklmnopqrstuvwxyz0123456789AB";

#[test]
fn a_token_is_accepted_and_never_debugged() {
    let token = BotToken::from_source(VARIABLE, Some(VALID.to_string()))
        .expect("a valid token is accepted");

    assert_eq!(token.expose(), VALID);

    let debug = format!("{token:?}");
    assert!(!debug.contains(VALID), "Debug leaked the token: {debug}");
    assert!(debug.contains("redacted"), "unexpected Debug: {debug}");
}

#[test]
fn a_missing_variable_is_reported_by_name() {
    let error = BotToken::from_source(VARIABLE, None).expect_err("a missing token is an error");
    let message = error.to_string();
    assert!(message.contains(VARIABLE), "unexpected message: {message}");
    assert!(message.contains("missing"), "unexpected message: {message}");
}

#[test]
fn unusable_values_are_rejected() {
    let cases: Vec<(String, &str)> = vec![
        ("abc.def".to_string(), "shorter"),
        (" ".to_string(), "whitespace"),
        ("a".repeat(60), "does not look like"),
        (
            "MTAwMDAwMDAwMDAwMDAwMDAw.Ga4dEf.abcdefghijklmnopqrstuvwxyz0123456789AB ".to_string(),
            "whitespace",
        ),
        (
            "Bot MTAwMDAwMDAwMDAwMDAwMDAw.Ga4dEf.abcdefghijklmnopqrstuvwxyz0123456789".to_string(),
            "whitespace",
        ),
        (
            "this-is-a-changeme-placeholder-token-with-dots.a.bcdefghijklmnop".to_string(),
            "placeholder",
        ),
    ];

    for (value, expected) in cases {
        let error =
            BotToken::from_source(VARIABLE, Some(value)).expect_err("this value must be rejected");
        let message = error.to_string();
        assert!(
            message.contains(expected),
            "expected `{expected}` in rejection message, got: {message}"
        );
    }
}
