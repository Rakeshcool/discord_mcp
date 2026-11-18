//! Permission names for channel overwrites.
//!
//! A tool never accepts a raw bitfield: the model names permissions, and this
//! module resolves them against Discord's documented bits. An unknown name is an
//! argument error, and so is `ADMINISTRATOR` in an `allow` list — granting it
//! through an overwrite is a privilege escalation the policy layer does not
//! model.

use crate::error::AppError;

/// Discord's permission bits, by documented name.
pub const NAMED: [(&str, u64); 41] = [
    ("CREATE_INSTANT_INVITE", 1 << 0),
    ("KICK_MEMBERS", 1 << 1),
    ("BAN_MEMBERS", 1 << 2),
    ("ADMINISTRATOR", 1 << 3),
    ("MANAGE_CHANNELS", 1 << 4),
    ("MANAGE_GUILD", 1 << 5),
    ("ADD_REACTIONS", 1 << 6),
    ("VIEW_AUDIT_LOG", 1 << 7),
    ("PRIORITY_SPEAKER", 1 << 8),
    ("STREAM", 1 << 9),
    ("VIEW_CHANNEL", 1 << 10),
    ("SEND_MESSAGES", 1 << 11),
    ("SEND_TTS_MESSAGES", 1 << 12),
    ("MANAGE_MESSAGES", 1 << 13),
    ("EMBED_LINKS", 1 << 14),
    ("ATTACH_FILES", 1 << 15),
    ("READ_MESSAGE_HISTORY", 1 << 16),
    ("MENTION_EVERYONE", 1 << 17),
    ("USE_EXTERNAL_EMOJIS", 1 << 18),
    ("VIEW_GUILD_INSIGHTS", 1 << 19),
    ("CONNECT", 1 << 20),
    ("SPEAK", 1 << 21),
    ("MUTE_MEMBERS", 1 << 22),
    ("DEAFEN_MEMBERS", 1 << 23),
    ("MOVE_MEMBERS", 1 << 24),
    ("USE_VAD", 1 << 25),
    ("CHANGE_NICKNAME", 1 << 26),
    ("MANAGE_NICKNAMES", 1 << 27),
    ("MANAGE_ROLES", 1 << 28),
    ("MANAGE_WEBHOOKS", 1 << 29),
    ("MANAGE_GUILD_EXPRESSIONS", 1 << 30),
    ("USE_APPLICATION_COMMANDS", 1 << 31),
    ("REQUEST_TO_SPEAK", 1 << 32),
    ("MANAGE_EVENTS", 1 << 33),
    ("MANAGE_THREADS", 1 << 34),
    ("CREATE_PUBLIC_THREADS", 1 << 35),
    ("CREATE_PRIVATE_THREADS", 1 << 36),
    ("USE_EXTERNAL_STICKERS", 1 << 37),
    ("SEND_MESSAGES_IN_THREADS", 1 << 38),
    ("USE_EMBEDDED_ACTIVITIES", 1 << 39),
    ("MODERATE_MEMBERS", 1 << 40),
];

/// A resolved set of permission bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PermissionBits(u64);

impl PermissionBits {
    /// The raw bits, for the adapter.
    pub const fn bits(self) -> u64 {
        self.0
    }
}

/// Resolve names into bits.
///
/// `allow_administrator` is false for `allow` lists and true for `deny` lists:
/// denying `ADMINISTRATOR` in one channel is a restriction, granting it is not.
pub fn resolve(
    names: &[String],
    context: &str,
    allow_administrator: bool,
) -> Result<PermissionBits, AppError> {
    let mut bits = 0u64;
    for raw in names {
        let name = raw.trim().to_ascii_uppercase();
        let Some((_, bit)) = NAMED.iter().find(|(known, _)| *known == name) else {
            return Err(AppError::invalid_argument(format!(
                "`{context}` contains `{raw}`, which is not a Discord permission name"
            )));
        };
        if name == "ADMINISTRATOR" && !allow_administrator {
            return Err(AppError::invalid_argument(format!(
                "`{context}` must not grant ADMINISTRATOR; it would escalate privileges"
            )));
        }
        bits |= bit;
    }
    Ok(PermissionBits(bits))
}

#[cfg(test)]
mod tests {
    use super::{PermissionBits, resolve};

    #[test]
    fn names_resolve_to_discord_bits() {
        let bits = resolve(
            &["VIEW_CHANNEL".to_string(), "send_messages".to_string()],
            "allow",
            false,
        )
        .expect("both names are known");
        assert_eq!(bits.bits(), (1 << 10) | (1 << 11));
    }

    #[test]
    fn an_unknown_name_is_an_argument_error() {
        let error = resolve(&["SEND_MESSAGE".to_string()], "allow", false)
            .expect_err("a typo must not resolve");
        assert!(error.message().contains("SEND_MESSAGE"), "got {error}");
    }

    #[test]
    fn administrator_cannot_be_granted_but_can_be_denied() {
        let error = resolve(&["ADMINISTRATOR".to_string()], "allow", false)
            .expect_err("an overwrite must not grant administrator");
        assert!(error.message().contains("ADMINISTRATOR"), "got {error}");

        let denied = resolve(&["ADMINISTRATOR".to_string()], "deny", true)
            .expect("denying it is a restriction");
        assert_eq!(denied.bits(), 1 << 3);
    }

    #[test]
    fn an_empty_list_is_empty() {
        let bits = resolve(&[], "allow", false).expect("nothing to resolve");
        assert_eq!(bits, PermissionBits::default());
        assert_eq!(bits.bits(), 0);
    }
}
