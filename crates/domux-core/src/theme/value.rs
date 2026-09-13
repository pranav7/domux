//! A role's value as a theme file writes it.

use domux_term::Rgb;
use std::fmt;

/// One role's value in a theme layer, before it is painted against the terminal's answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorValue {
    /// That colour.
    Hex(Rgb),
    /// The terminal's own ground, drawn as its default colour. Only a ground takes it.
    Default,
    /// The terminal's background.
    Background,
    /// The terminal's foreground.
    Foreground,
    /// The terminal's palette slot, 0 to 15.
    Palette(u8),
    /// The background moved `n/d` of the way to the foreground.
    Blend { n: u8, d: u8 },
    /// The background moved `n/d` of the way away from the foreground.
    Shade { n: u8, d: u8 },
}

/// Why a string is not a colour value. `Display` is the reason a warning gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueError {
    /// Not one of the forms at all.
    Form,
    /// Starts with `#` but is not six hex digits.
    Hex,
    /// A palette slot that is not a number from 0 to 15.
    Slot,
    /// A fraction that is not `n/d` with `d` from 1 to 255, written by a blend or a shade.
    Fraction(&'static str),
    /// A fraction past 1.
    PastOne(&'static str),
    /// A percent, which no form takes.
    Percent(&'static str),
}

impl fmt::Display for ValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValueError::Form => write!(
                f,
                "a colour is a hex colour, default, background, foreground, palette N, blend n/d or shade n/d"
            ),
            ValueError::Hex => write!(f, "a hex colour is # and six hex digits"),
            ValueError::Slot => write!(f, "a palette slot is 0 to 15"),
            ValueError::Fraction(word) => {
                write!(f, "a {word} is n/d with d from 1 to 255")
            }
            ValueError::PastOne(word) => write!(f, "a {word} is 0 to 1"),
            ValueError::Percent(word) => write!(f, "a {word} is a fraction n/d, not a percent"),
        }
    }
}

impl ColorValue {
    /// Parses one of the string forms: `#rrggbb`, `default`, `background`, `foreground`,
    /// `palette N`, `blend n/d` or `shade n/d`.
    pub fn parse(text: &str) -> Result<ColorValue, ValueError> {
        let text = text.trim_matches(' ');
        if let Some(digits) = text.strip_prefix('#') {
            return parse_hex(digits).map(ColorValue::Hex);
        }
        let tokens: Vec<&str> = text.split(' ').filter(|t| !t.is_empty()).collect();
        match tokens.as_slice() {
            ["default"] => Ok(ColorValue::Default),
            ["background"] => Ok(ColorValue::Background),
            ["foreground"] => Ok(ColorValue::Foreground),
            ["palette", rest @ ..] => match rest {
                [slot] => match number(slot) {
                    Some(n) if n <= 15 => Ok(ColorValue::Palette(n as u8)),
                    _ => Err(ValueError::Slot),
                },
                _ => Err(ValueError::Slot),
            },
            [word @ ("blend" | "shade"), rest @ ..] => {
                let word: &'static str = if *word == "blend" { "blend" } else { "shade" };
                let (n, d) = fraction(word, rest)?;
                Ok(if word == "blend" {
                    ColorValue::Blend { n, d }
                } else {
                    ColorValue::Shade { n, d }
                })
            }
            _ => Err(ValueError::Form),
        }
    }
}

fn parse_hex(digits: &str) -> Result<Rgb, ValueError> {
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ValueError::Hex);
    }
    let channel = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).map_err(|_| ValueError::Hex);
    Ok(Rgb {
        r: channel(0)?,
        g: channel(2)?,
        b: channel(4)?,
    })
}

/// A number written in ASCII digits only, so `+4` is not one. `None` past `u32`.
fn number(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn fraction(word: &'static str, rest: &[&str]) -> Result<(u8, u8), ValueError> {
    if rest.iter().any(|t| t.contains('%')) {
        return Err(ValueError::Percent(word));
    }
    let [one] = rest else {
        return Err(ValueError::Fraction(word));
    };
    let Some((n, d)) = one.split_once('/') else {
        return Err(ValueError::Fraction(word));
    };
    match (number(n), number(d)) {
        (Some(n), Some(d)) if (1..=255).contains(&d) => {
            if n > d {
                Err(ValueError::PastOne(word))
            } else {
                Ok((n as u8, d as u8))
            }
        }
        _ => Err(ValueError::Fraction(word)),
    }
}

impl fmt::Display for ColorValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ColorValue::Hex(Rgb { r, g, b }) => write!(f, "#{r:02x}{g:02x}{b:02x}"),
            ColorValue::Default => write!(f, "default"),
            ColorValue::Background => write!(f, "background"),
            ColorValue::Foreground => write!(f, "foreground"),
            ColorValue::Palette(slot) => write!(f, "palette {slot}"),
            ColorValue::Blend { n, d } => write!(f, "blend {n}/{d}"),
            ColorValue::Shade { n, d } => write!(f, "shade {n}/{d}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }

    #[test]
    fn a_colour_value_parses_hex_default_background_foreground_palette_blend_and_shade() {
        let cases = [
            ("#f38d70", ColorValue::Hex(rgb(0xf3, 0x8d, 0x70)), "#f38d70"),
            ("#F38D70", ColorValue::Hex(rgb(0xf3, 0x8d, 0x70)), "#f38d70"),
            ("default", ColorValue::Default, "default"),
            ("background", ColorValue::Background, "background"),
            ("foreground", ColorValue::Foreground, "foreground"),
            ("palette 0", ColorValue::Palette(0), "palette 0"),
            ("palette 15", ColorValue::Palette(15), "palette 15"),
            ("  palette   12 ", ColorValue::Palette(12), "palette 12"),
            ("blend 1/9", ColorValue::Blend { n: 1, d: 9 }, "blend 1/9"),
            ("blend 0/1", ColorValue::Blend { n: 0, d: 1 }, "blend 0/1"),
            (
                "blend 255/255",
                ColorValue::Blend { n: 255, d: 255 },
                "blend 255/255",
            ),
            ("shade 1/5", ColorValue::Shade { n: 1, d: 5 }, "shade 1/5"),
            (
                " shade  11/18",
                ColorValue::Shade { n: 11, d: 18 },
                "shade 11/18",
            ),
        ];
        for (text, want, shown) in cases {
            let got = ColorValue::parse(text);
            assert_eq!(got, Ok(want), "{text:?}");
            assert_eq!(want.to_string(), shown, "{text:?}");
            assert_eq!(ColorValue::parse(shown), Ok(want), "{shown:?} round trip");
        }
    }

    #[test]
    fn a_colour_value_refuses_a_slot_past_15_a_fraction_past_1_a_short_hex_and_a_percent() {
        let cases = [
            ("palette 16", ValueError::Slot),
            ("palette 255", ValueError::Slot),
            ("palette 99999", ValueError::Slot),
            ("palette +4", ValueError::Slot),
            ("palette", ValueError::Slot),
            ("palette 1 2", ValueError::Slot),
            ("blend 10/9", ValueError::PastOne("blend")),
            ("shade 2/1", ValueError::PastOne("shade")),
            ("blend 1/0", ValueError::Fraction("blend")),
            ("blend 1/256", ValueError::Fraction("blend")),
            ("blend 1/99999", ValueError::Fraction("blend")),
            ("blend 1 / 9", ValueError::Fraction("blend")),
            ("blend 0.5", ValueError::Fraction("blend")),
            ("blend", ValueError::Fraction("blend")),
            ("shade -1/5", ValueError::Fraction("shade")),
            ("#f38d7", ValueError::Hex),
            ("#f38d700", ValueError::Hex),
            ("#fff", ValueError::Hex),
            ("#g38d70", ValueError::Hex),
            ("f38d70", ValueError::Form),
            ("blend 50%", ValueError::Percent("blend")),
            ("shade 20%", ValueError::Percent("shade")),
            ("Default", ValueError::Form),
            ("BLEND 1/9", ValueError::Form),
            ("", ValueError::Form),
            ("background 1", ValueError::Form),
            ("\tdefault", ValueError::Form),
        ];
        for (text, want) in cases {
            assert_eq!(ColorValue::parse(text), Err(want), "{text:?}");
        }
        assert_eq!(
            ValueError::PastOne("blend").to_string(),
            "a blend is 0 to 1"
        );
    }
}
