//! Typed Discord snowflake identifiers.
//!
//! Discord snowflakes are 64-bit unsigned integers serialized as decimal
//! strings. They are validated as strings and held as `u64` here, never as
//! floats: a 19-digit ID does not survive an `f64` round trip, and a JSON
//! parser that guesses numbers would silently corrupt it.
//!
//! Distinct types for guild, channel, message and user IDs mean an argument
//! cannot be passed to the wrong operation by accident. Parsing happens at the
//! tool boundary, so a malformed ID is rejected before a Discord request is
//! built.

use std::fmt;

use crate::config::schema::is_snowflake_like;
use crate::error::AppError;

macro_rules! snowflake_type {
    ($name:ident, $field:literal) => {
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);

        impl $name {
            /// Parse a snowflake supplied by a caller, rejecting anything that
            /// is not a Discord identifier.
            pub fn parse(value: &str) -> Result<Self, AppError> {
                let value = value.trim();
                let invalid = || {
                    AppError::invalid_argument(format!(
                        "`{}` must be a Discord snowflake ID (digits only, at most 20, no leading zero)",
                        $field
                    ))
                };
                if !is_snowflake_like(value) {
                    return Err(invalid());
                }
                value.parse::<u64>().map(Self).map_err(|_| invalid())
            }

            /// Wrap an identifier that Discord itself returned.
            pub fn from_u64(value: u64) -> Self {
                Self(value)
            }

            /// The numeric identifier.
            pub const fn get(self) -> u64 {
                self.0
            }

            /// The identifier in Discord's decimal string form.
            pub fn as_string(self) -> String {
                self.0.to_string()
            }
        }

        /// Serialized as Discord's decimal string, so a 19-digit id survives any
        /// JSON consumer that would lose precision on a bare number.
        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(&self.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}", self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}({})", stringify!($name), self.0)
            }
        }
    };
}

snowflake_type!(GuildId, "guild_id");
snowflake_type!(ChannelId, "channel_id");
snowflake_type!(MessageId, "message_id");
snowflake_type!(UserId, "user_id");
