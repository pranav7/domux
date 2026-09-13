//! Colour arithmetic: exact integer mixing and WCAG 2 luminance and contrast.

use domux_term::Rgb;

/// `a` moved `n/d` of the way to `b`, per channel, rounded half away from zero, in exact
/// integer arithmetic: `(2·(a·d + (b − a)·n) + d) / (2·d)`. Floating point rounds some of these
/// the other way.
pub fn mix(a: Rgb, b: Rgb, n: u8, d: u8) -> Rgb {
    let (n, d) = (i32::from(n), i32::from(d));
    let channel = |a: u8, b: u8| {
        let (a, b) = (i32::from(a), i32::from(b));
        // a·d + (b − a)·n is a·(d − n) + b·n, never negative for n ≤ d, so floor division of the
        // doubled value plus d rounds half away from zero.
        let value = (2 * (a * d + (b - a) * n) + d) / (2 * d);
        value.clamp(0, 255) as u8
    };
    Rgb {
        r: channel(a.r, b.r),
        g: channel(a.g, b.g),
        b: channel(a.b, b.b),
    }
}

/// WCAG 2 relative luminance, 0 for black and 1 for white.
pub fn luminance(c: Rgb) -> f64 {
    fn linear(channel: u8) -> f64 {
        let v = f64::from(channel) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
}

/// WCAG 2 contrast ratio, 1 to 21, the same whichever colour is given first.
pub fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (light, dark) = if la >= lb { (la, lb) } else { (lb, la) };
    (light + 0.05) / (dark + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(v: u32) -> Rgb {
        Rgb {
            r: (v >> 16) as u8,
            g: (v >> 8) as u8,
            b: v as u8,
        }
    }

    #[test]
    fn mix_is_exact_integer_arithmetic_rounded_half_away_from_zero() {
        assert_eq!(mix(hex(0x2d0000), hex(0x000000), 11, 18), hex(0x120000));
        assert_eq!(mix(hex(0x000000), hex(0x0103ff), 1, 2), hex(0x010280));
        assert_eq!(mix(hex(0x123456), hex(0xabcdef), 0, 7), hex(0x123456));
        assert_eq!(mix(hex(0x123456), hex(0xabcdef), 7, 7), hex(0xabcdef));
        assert_eq!(mix(hex(0xffffff), hex(0x000000), 1, 255), hex(0xfefefe));
    }

    #[test]
    fn blends_of_mocha_in_ninths_are_the_domux_neutrals() {
        let base = hex(0x1e1e2e);
        let text = hex(0xcdd6f4);
        let ninths = [
            (1, 0x313244),
            (2, 0x45475a),
            (3, 0x585b70),
            (4, 0x6c7086),
            (5, 0x7f849c),
            (7, 0xa6adc8),
        ];
        for (n, want) in ninths {
            assert_eq!(mix(base, text, n, 9), hex(want), "{n}/9");
        }
    }

    #[test]
    fn shade_one_fifth_of_mocha_is_mantle() {
        assert_eq!(mix(hex(0x1e1e2e), hex(0x000000), 1, 5), hex(0x181825));
    }

    #[test]
    fn contrast_of_black_on_white_is_21() {
        let black = hex(0x000000);
        let white = hex(0xffffff);
        assert!((contrast(black, white) - 21.0).abs() < 1e-9);
        assert!((contrast(white, black) - 21.0).abs() < 1e-9);
        assert!((contrast(white, white) - 1.0).abs() < 1e-9);
        assert!((luminance(white) - 1.0).abs() < 1e-9);
        assert_eq!(luminance(black), 0.0);
    }
}
