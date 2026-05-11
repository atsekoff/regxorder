use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A platform-agnostic hotkey modifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyModifier {
    Control,
    Alt,
    Shift,
    Win,
}

impl HotkeyModifier {
    /// Returns the user-facing name of this modifier.
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Control => "Ctrl",
            Self::Alt => "Alt",
            Self::Shift => "Shift",
            Self::Win => "Win",
        }
    }

    const fn canonical_token(self) -> &'static str {
        match self {
            Self::Control => "ctrl",
            Self::Alt => "alt",
            Self::Shift => "shift",
            Self::Win => "win",
        }
    }

    const fn sort_rank(self) -> u8 {
        match self {
            Self::Control => 0,
            Self::Alt => 1,
            Self::Shift => 2,
            Self::Win => 3,
        }
    }

    fn parse_token(token: &str) -> Option<Self> {
        if token.eq_ignore_ascii_case("ctrl") || token.eq_ignore_ascii_case("control") {
            Some(Self::Control)
        } else if token.eq_ignore_ascii_case("alt") {
            Some(Self::Alt)
        } else if token.eq_ignore_ascii_case("shift") {
            Some(Self::Shift)
        } else if token.eq_ignore_ascii_case("win") || token.eq_ignore_ascii_case("windows") {
            Some(Self::Win)
        } else {
            None
        }
    }
}

impl fmt::Display for HotkeyModifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.display_name())
    }
}

/// A platform-agnostic hotkey base key supported by the V1 global hotkey model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyKey {
    Escape,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}

impl HotkeyKey {
    /// Returns the user-facing name of this hotkey key.
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Escape => "Escape",
            Self::F1 => "F1",
            Self::F2 => "F2",
            Self::F3 => "F3",
            Self::F4 => "F4",
            Self::F5 => "F5",
            Self::F6 => "F6",
            Self::F7 => "F7",
            Self::F8 => "F8",
            Self::F9 => "F9",
            Self::F10 => "F10",
            Self::F11 => "F11",
            Self::F12 => "F12",
        }
    }

    fn parse_token(token: &str) -> Option<Self> {
        if token.eq_ignore_ascii_case("esc") || token.eq_ignore_ascii_case("escape") {
            Some(Self::Escape)
        } else if token.eq_ignore_ascii_case("f1") {
            Some(Self::F1)
        } else if token.eq_ignore_ascii_case("f2") {
            Some(Self::F2)
        } else if token.eq_ignore_ascii_case("f3") {
            Some(Self::F3)
        } else if token.eq_ignore_ascii_case("f4") {
            Some(Self::F4)
        } else if token.eq_ignore_ascii_case("f5") {
            Some(Self::F5)
        } else if token.eq_ignore_ascii_case("f6") {
            Some(Self::F6)
        } else if token.eq_ignore_ascii_case("f7") {
            Some(Self::F7)
        } else if token.eq_ignore_ascii_case("f8") {
            Some(Self::F8)
        } else if token.eq_ignore_ascii_case("f9") {
            Some(Self::F9)
        } else if token.eq_ignore_ascii_case("f10") {
            Some(Self::F10)
        } else if token.eq_ignore_ascii_case("f11") {
            Some(Self::F11)
        } else if token.eq_ignore_ascii_case("f12") {
            Some(Self::F12)
        } else {
            None
        }
    }
}

impl fmt::Display for HotkeyKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.display_name())
    }
}

/// Errors produced while parsing or constructing a hotkey binding.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HotkeyParseError {
    #[error("hotkeys cannot contain empty segments")]
    EmptySegment,

    #[error("hotkey modifier `{token}` was provided more than once")]
    DuplicateModifier { token: String },

    #[error("hotkeys can only contain one base key such as escape or f1-f12")]
    MultipleBaseKeys,

    #[error(
        "unsupported hotkey token `{token}`; use modifiers like ctrl/alt/shift/win and keys like escape or f1-f12"
    )]
    UnsupportedToken { token: String },

    #[error("hotkeys require at least one modifier such as ctrl, alt, shift, or win")]
    MissingModifier,

    #[error("hotkeys require a base key such as escape or f1-f12")]
    MissingBaseKey,
}

/// A parsed hotkey chord that can be shared across CLI, SDK, and UI layers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeyBinding {
    modifiers: Vec<HotkeyModifier>,
    key: HotkeyKey,
}

impl HotkeyBinding {
    /// Creates a validated hotkey binding from explicit modifier and key values.
    pub fn new(
        mut modifiers: Vec<HotkeyModifier>,
        key: HotkeyKey,
    ) -> Result<Self, HotkeyParseError> {
        if modifiers.is_empty() {
            return Err(HotkeyParseError::MissingModifier);
        }

        for (index, modifier) in modifiers.iter().enumerate() {
            if modifiers[..index].contains(modifier) {
                return Err(HotkeyParseError::DuplicateModifier {
                    token: modifier.canonical_token().to_string(),
                });
            }
        }

        modifiers.sort_unstable_by_key(|modifier| modifier.sort_rank());

        Ok(Self { modifiers, key })
    }

    /// Returns the modifiers that make up this hotkey binding.
    pub fn modifiers(&self) -> &[HotkeyModifier] {
        &self.modifiers
    }

    /// Returns the base key used by this hotkey binding.
    pub const fn key(&self) -> HotkeyKey {
        self.key
    }
}

impl fmt::Display for HotkeyBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for modifier in &self.modifiers {
            write!(formatter, "{}+", modifier)?;
        }

        formatter.write_str(self.key.display_name())
    }
}

impl FromStr for HotkeyBinding {
    type Err = HotkeyParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut modifiers = Vec::new();
        let mut key = None;

        for token in value.split('+').map(str::trim) {
            if token.is_empty() {
                return Err(HotkeyParseError::EmptySegment);
            }

            if let Some(parsed_modifier) = HotkeyModifier::parse_token(token) {
                if modifiers.contains(&parsed_modifier) {
                    return Err(HotkeyParseError::DuplicateModifier {
                        token: token.to_string(),
                    });
                }

                modifiers.push(parsed_modifier);
                continue;
            }

            if let Some(parsed_key) = HotkeyKey::parse_token(token) {
                if key.replace(parsed_key).is_some() {
                    return Err(HotkeyParseError::MultipleBaseKeys);
                }

                continue;
            }

            return Err(HotkeyParseError::UnsupportedToken {
                token: token.to_string(),
            });
        }

        let key = key.ok_or(HotkeyParseError::MissingBaseKey)?;
        Self::new(modifiers, key)
    }
}

#[cfg(test)]
mod tests {
    use super::{HotkeyBinding, HotkeyKey, HotkeyModifier, HotkeyParseError};

    #[test]
    fn hotkey_bindings_parse_case_insensitive_chords() {
        let hotkey = "ctrl+shift+f9"
            .parse::<HotkeyBinding>()
            .expect("hotkey bindings should parse");

        assert_eq!(hotkey.to_string(), "Ctrl+Shift+F9");
    }

    #[test]
    fn hotkey_bindings_accept_escape_aliases() {
        let hotkey = "control+alt+esc"
            .parse::<HotkeyBinding>()
            .expect("hotkey bindings should parse escape aliases");

        assert_eq!(hotkey.to_string(), "Ctrl+Alt+Escape");
    }

    #[test]
    fn hotkey_bindings_require_a_modifier() {
        let error = "f9"
            .parse::<HotkeyBinding>()
            .expect_err("hotkey bindings should reject missing modifiers");

        assert_eq!(error, HotkeyParseError::MissingModifier);
    }

    #[test]
    fn hotkey_bindings_reject_duplicate_modifiers() {
        let error = "ctrl+ctrl+f9"
            .parse::<HotkeyBinding>()
            .expect_err("hotkey bindings should reject duplicate modifiers");

        assert_eq!(
            error,
            HotkeyParseError::DuplicateModifier {
                token: String::from("ctrl")
            }
        );
    }

    #[test]
    fn hotkey_bindings_canonicalize_modifier_order() {
        let hotkey = HotkeyBinding::new(
            vec![HotkeyModifier::Shift, HotkeyModifier::Control],
            HotkeyKey::F9,
        )
        .expect("hotkey bindings should be constructible");

        assert_eq!(hotkey.to_string(), "Ctrl+Shift+F9");
    }
}
