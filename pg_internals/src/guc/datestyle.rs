//! Rust implementation of PostgreSQL's `check_datestyle` and identifier-list rules.

use std::fmt;

use crate::constants::NAMEDATALEN;

/// The argument containing an invalid DateStyle setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateStyleInput {
    Value,
    Current,
    Reset,
}

impl fmt::Display for DateStyleInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Value => "value",
            Self::Current => "current setting",
            Self::Reset => "reset setting",
        })
    }
}

/// A malformed DateStyle value or context.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DateStyleError {
    #[error("invalid DateStyle {input}: {detail}")]
    Invalid {
        input: DateStyleInput,
        /// PostgreSQL's check-hook error detail.
        detail: String,
    },
    #[error("DateStyle {input} contains a NUL byte")]
    InteriorNul { input: DateStyleInput },
}

/// Normalize a DateStyle assignment using the session's current and reset values.
///
/// Missing components inherit the current setting; `DEFAULT` uses the reset
/// setting. Pass the complete settings reported by PostgreSQL when available.
/// Partial reset settings inherit `ISO, MDY`; partial current settings inherit
/// the normalized reset setting. No session state is mutated.
///
/// Parsing follows PostgreSQL's UTF-8 identifier rules, including ASCII case
/// folding, quoted identifiers, and truncation to `NAMEDATALEN - 1` bytes.
///
/// # Errors
/// Returns an error identifying the argument with invalid syntax, unrecognized
/// keywords, conflicting components, or an interior NUL byte.
///
/// # Examples
/// ```
/// use pg_internals::guc::normalize_datestyle;
/// assert_eq!(normalize_datestyle("MDY", "ISO, YMD", "SQL, DMY")?, "ISO, MDY");
/// assert_eq!(normalize_datestyle("DEFAULT, ISO", "German, DMY", "SQL, YMD")?, "ISO, YMD");
/// # Ok::<(), pg_internals::guc::DateStyleError>(())
/// ```
pub fn normalize_datestyle(
    value: &str,
    current: &str,
    reset: &str,
) -> Result<String, DateStyleError> {
    for (text, input) in [
        (value, DateStyleInput::Value),
        (current, DateStyleInput::Current),
        (reset, DateStyleInput::Reset),
    ] {
        if text.contains('\0') {
            return Err(DateStyleError::InteriorNul { input });
        }
    }
    let boot = DateStyle {
        format: Format::Iso,
        order: Order::Mdy,
    };
    let reset = boot.apply(reset, boot, DateStyleInput::Reset)?;
    let current = reset.apply(current, reset, DateStyleInput::Current)?;
    let result = current.apply(value, reset, DateStyleInput::Value)?;
    Ok(result.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Iso,
    Sql,
    Postgres,
    German,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Order {
    Ymd,
    Dmy,
    Mdy,
}

#[derive(Debug, Clone, Copy)]
struct DateStyle {
    format: Format,
    order: Order,
}

impl fmt::Display for DateStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let format = match self.format {
            Format::Iso => "ISO",
            Format::Sql => "SQL",
            Format::Postgres => "Postgres",
            Format::German => "German",
        };
        let order = match self.order {
            Order::Ymd => "YMD",
            Order::Dmy => "DMY",
            Order::Mdy => "MDY",
        };
        write!(f, "{format}, {order}")
    }
}

impl DateStyle {
    fn apply(
        mut self,
        value: &str,
        reset: Self,
        input: DateStyleInput,
    ) -> Result<Self, DateStyleError> {
        let invalid = |detail: String| DateStyleError::Invalid { input, detail };
        // Parse the entire list first: syntax errors take precedence over keywords.
        let tokens = identifiers(value).map_err(|()| invalid("List syntax is invalid.".into()))?;
        let mut have_format = false;
        let mut have_order = false;
        let mut conflict = false;
        for token in tokens {
            let keyword = token.to_ascii_lowercase();
            let format = match keyword.as_str() {
                "iso" => Some(Format::Iso),
                "sql" => Some(Format::Sql),
                "german" => Some(Format::German),
                s if s.starts_with("postgres") => Some(Format::Postgres),
                _ => None,
            };
            if let Some(format) = format {
                conflict |= have_format && self.format != format;
                self.format = format;
                have_format = true;
                if format == Format::German && !have_order {
                    self.order = Order::Dmy;
                }
                continue;
            }
            let order = match keyword.as_str() {
                "ymd" => Some(Order::Ymd),
                "dmy" => Some(Order::Dmy),
                "mdy" | "us" => Some(Order::Mdy),
                s if s.starts_with("euro") => Some(Order::Dmy),
                s if s.starts_with("noneuro") => Some(Order::Mdy),
                _ => None,
            };
            if let Some(order) = order {
                conflict |= have_order && self.order != order;
                self.order = order;
                have_order = true;
            } else if keyword == "default" {
                // DEFAULT and German's implied order don't count as explicit
                // components, so later explicit components can override them.
                if !have_format {
                    self.format = reset.format;
                }
                if !have_order {
                    self.order = reset.order;
                }
            } else {
                return Err(invalid(format!("Unrecognized key word: \"{token}\".")));
            }
        }
        if conflict {
            return Err(invalid("Conflicting \"DateStyle\" specifications.".into()));
        }
        Ok(self)
    }
}

// PostgreSQL scanner whitespace includes vertical tab, unlike Rust's
// is_ascii_whitespace, and excludes Unicode whitespace.
fn scanner_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c')
}

fn identifiers(value: &str) -> Result<Vec<String>, ()> {
    let mut chars = value.chars().peekable();
    let mut tokens = Vec::new();
    while chars.peek().copied().is_some_and(scanner_space) {
        chars.next();
    }
    if chars.peek().is_none() {
        return Ok(tokens);
    }
    loop {
        let mut token = String::new();
        if chars.peek() == Some(&'"') {
            chars.next();
            loop {
                match chars.next() {
                    Some('"') if chars.peek() == Some(&'"') => {
                        chars.next();
                        token.push('"');
                    }
                    Some('"') => break,
                    Some(c) => token.push(c),
                    None => return Err(()),
                }
            }
        } else {
            while let Some(c) = chars.peek().copied() {
                if c == ',' || scanner_space(c) {
                    break;
                }
                chars.next();
                token.push(c.to_ascii_lowercase());
            }
            if token.is_empty() {
                return Err(());
            }
        }
        let mut len = token.len().min(NAMEDATALEN as usize - 1);
        while !token.is_char_boundary(len) {
            len -= 1;
        }
        token.truncate(len);
        tokens.push(token);
        while chars.peek().copied().is_some_and(scanner_space) {
            chars.next();
        }
        match chars.next() {
            None => return Ok(tokens),
            Some(',') => {
                while chars.peek().copied().is_some_and(scanner_space) {
                    chars.next();
                }
            }
            Some(_) => return Err(()),
        }
    }
}
