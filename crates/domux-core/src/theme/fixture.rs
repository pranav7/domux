//! The answers of the 22 themes Omarchy ships: each theme's background, foreground and palette
//! slots 0 to 15, as a terminal drawing that theme answers them. Copied from the theme files under
//! /usr/share/omarchy/themes, one directory per theme. The guards' tests paint the built-in
//! terminal theme against every one of them.

use super::TerminalColors;
use domux_term::Rgb;

/// One Omarchy theme's answers, named after the directory it came from.
pub struct OmarchyTheme {
    pub name: &'static str,
    pub background: u32,
    pub foreground: u32,
    pub palette: [u32; 16],
}

impl OmarchyTheme {
    /// What a terminal drawing this theme answers: every colour.
    pub fn colors(&self) -> TerminalColors {
        TerminalColors {
            fg: Some(rgb(self.foreground)),
            bg: Some(rgb(self.background)),
            palette: self.palette.map(|v| Some(rgb(v))),
        }
    }
}

fn rgb(v: u32) -> Rgb {
    Rgb {
        r: (v >> 16) as u8,
        g: (v >> 8) as u8,
        b: v as u8,
    }
}

/// The theme of that name.
pub fn theme(name: &str) -> &'static OmarchyTheme {
    OMARCHY
        .iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("no Omarchy theme {name} in the fixture"))
}

/// Every theme Omarchy ships, in the order of their directories.
pub const OMARCHY: [OmarchyTheme; 22] = [
    OmarchyTheme {
        name: "catppuccin",
        background: 0x1e1e2e,
        foreground: 0xcdd6f4,
        palette: [
            0x1e1e2e, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xcdd6f4,
            0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xcdd6f4,
        ],
    },
    OmarchyTheme {
        name: "catppuccin-latte",
        background: 0xeff1f5,
        foreground: 0x4c4f69,
        palette: [
            0xeff1f5, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0x4c4f69,
            0xacb0be, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0x4c4f69,
        ],
    },
    OmarchyTheme {
        name: "ethereal",
        background: 0x060b1e,
        foreground: 0xffcead,
        palette: [
            0x060b1e, 0xed5b5a, 0x92a593, 0xe9bb4f, 0x7d82d9, 0xc89dc1, 0xa3bfd1, 0xffcead,
            0x6d7db6, 0xfaaaa9, 0xc4cfc4, 0xf7dc9c, 0xc2c4f0, 0xead7e7, 0xdfeaf0, 0xffcead,
        ],
    },
    OmarchyTheme {
        name: "everforest",
        background: 0x2d353b,
        foreground: 0xd3c6aa,
        palette: [
            0x2d353b, 0xe67e80, 0xa7c080, 0xdbbc7f, 0x7fbbb3, 0xd699b6, 0x83c092, 0xd3c6aa,
            0x475258, 0xe67e80, 0xa7c080, 0xdbbc7f, 0x7fbbb3, 0xd699b6, 0x83c092, 0xd3c6aa,
        ],
    },
    OmarchyTheme {
        name: "flexoki-light",
        background: 0xfffcf0,
        foreground: 0x100f0f,
        palette: [
            0xfffcf0, 0xd14d41, 0x879a39, 0xd0a215, 0x205ea6, 0xce5d97, 0x3aa99f, 0x100f0f,
            0xb7b5ac, 0xd14d41, 0x879a39, 0xd0a215, 0x4385be, 0xce5d97, 0x3aa99f, 0x100f0f,
        ],
    },
    OmarchyTheme {
        name: "gruvbox",
        background: 0x282828,
        foreground: 0xd4be98,
        palette: [
            0x282828, 0xea6962, 0xa9b665, 0xd8a657, 0x7daea3, 0xd3869b, 0x89b482, 0xd4be98,
            0x665c54, 0xea6962, 0xa9b665, 0xd8a657, 0x7daea3, 0xd3869b, 0x89b482, 0xd4be98,
        ],
    },
    OmarchyTheme {
        name: "hackerman",
        background: 0x0b0c16,
        foreground: 0xddf7ff,
        palette: [
            0x0b0c16, 0x50f872, 0x4fe88f, 0x50f7d4, 0x829dd4, 0x86a7df, 0x7cf8f7, 0xddf7ff,
            0x2d3450, 0x85ff9d, 0x9cf7c2, 0xa4ffec, 0xc4d2ed, 0xcddbf4, 0xd1fffe, 0xddf7ff,
        ],
    },
    OmarchyTheme {
        name: "kanagawa",
        background: 0x1f1f28,
        foreground: 0xdcd7ba,
        palette: [
            0x1f1f28, 0xc34043, 0x76946a, 0xc0a36e, 0x7e9cd8, 0x957fb8, 0x6a9589, 0xdcd7ba,
            0x54546d, 0xe82424, 0x98bb6c, 0xe6c384, 0x7fb4ca, 0x938aa9, 0x7aa89f, 0xdcd7ba,
        ],
    },
    OmarchyTheme {
        name: "last-horizon",
        background: 0x0c0b0c,
        foreground: 0xfafcfb,
        palette: [
            0x0c0b0c, 0xc38b7b, 0x87a9b0, 0x6b5e73, 0xb59790, 0xc4d8e2, 0xa5a0b6, 0xfafcfb,
            0x584e51, 0xc38b7b, 0x87a9b0, 0x6b5e73, 0xb59790, 0xc4d8e2, 0xa5a0b6, 0xe2dddc,
        ],
    },
    OmarchyTheme {
        name: "lumon",
        background: 0x16242d,
        foreground: 0xd6e2ee,
        palette: [
            0x16242d, 0x4d86b0, 0x5e95bc, 0x6fa4c9, 0x6fb8e3, 0x8bc9eb, 0xb4e4f6, 0xd6e2ee,
            0x304860, 0x73a6cb, 0x86b7d8, 0x9dcae5, 0xf2fcff, 0xb1d8ee, 0xd1eef8, 0xf2fcff,
        ],
    },
    OmarchyTheme {
        name: "lupine",
        background: 0xfafafa,
        foreground: 0x212121,
        palette: [
            0xfafafa, 0xc900c4, 0x4a2fd0, 0x026fde, 0x3264eb, 0x8a4ad7, 0x0c67de, 0x212121,
            0x9e9e9e, 0xf930fb, 0x9f85e0, 0x358fff, 0x5482ff, 0xb363ff, 0x3986ff, 0x000000,
        ],
    },
    OmarchyTheme {
        name: "matte-black",
        background: 0x121212,
        foreground: 0xbebebe,
        palette: [
            0x121212, 0xd35f5f, 0xffc107, 0xb91c1c, 0xe68e0d, 0xd35f5f, 0xbebebe, 0xbebebe,
            0x333333, 0xb91c1c, 0xffc107, 0xb90a0a, 0xf59e0b, 0xb91c1c, 0xeaeaea, 0xbebebe,
        ],
    },
    OmarchyTheme {
        name: "miasma",
        background: 0x222222,
        foreground: 0xc2c2b0,
        palette: [
            0x222222, 0x685742, 0x5f875f, 0xb36d43, 0x78824b, 0xbb7744, 0xc9a554, 0xc2c2b0,
            0x666666, 0x685742, 0x5f875f, 0xb36d43, 0x78824b, 0xbb7744, 0xc9a554, 0xc2c2b0,
        ],
    },
    OmarchyTheme {
        name: "nord",
        background: 0x2e3440,
        foreground: 0xd8dee9,
        palette: [
            0x2e3440, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x88c0d0, 0xd8dee9,
            0x4c566a, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x8fbcbb, 0xd8dee9,
        ],
    },
    OmarchyTheme {
        name: "osaka-jade",
        background: 0x111c18,
        foreground: 0xc1c497,
        palette: [
            0x111c18, 0xff5345, 0x549e6a, 0x459451, 0x509475, 0xd2689c, 0x2dd5b7, 0xc1c497,
            0x53685b, 0xdb9f9c, 0x63b07a, 0xe5c736, 0xacd4cf, 0x75bbb3, 0x8cd3cb, 0xf7e8b2,
        ],
    },
    OmarchyTheme {
        name: "retro-82",
        background: 0x05182e,
        foreground: 0xf6dcac,
        palette: [
            0x05182e, 0xf85525, 0x028391, 0xe97b3c, 0x3f8f8a, 0x3f8f8a, 0x8cbfb8, 0xf6dcac,
            0x2a6b78, 0xf85525, 0x028391, 0xe97b3c, 0xfaa968, 0x3f8f8a, 0x8cbfb8, 0xf6dcac,
        ],
    },
    OmarchyTheme {
        name: "ristretto",
        background: 0x2c2525,
        foreground: 0xe6d9db,
        palette: [
            0x2c2525, 0xfd6883, 0xadda78, 0xf9cc6c, 0xf38d70, 0xa8a9eb, 0x85dacc, 0xe6d9db,
            0x72696a, 0xff8297, 0xc8e292, 0xfcd675, 0xf8a788, 0xbebffd, 0x9bf1e1, 0xe6d9db,
        ],
    },
    OmarchyTheme {
        name: "rose-pine",
        background: 0xfaf4ed,
        foreground: 0x575279,
        palette: [
            0xfaf4ed, 0xb4637a, 0x286983, 0xea9d34, 0x56949f, 0x907aa9, 0xd7827e, 0x575279,
            0xcecacd, 0xb4637a, 0x286983, 0xea9d34, 0x56949f, 0x907aa9, 0xd7827e, 0x575279,
        ],
    },
    OmarchyTheme {
        name: "solitude",
        background: 0x101315,
        foreground: 0xcacccc,
        palette: [
            0x101315, 0x565d60, 0x9fa5a9, 0xd9dbdc, 0x798186, 0xaeaeae, 0x707070, 0xcacccc,
            0x4b4e55, 0xde6145, 0x343d41, 0xc9c2b4, 0x5d6367, 0x9a9a9a, 0x707070, 0xa5aeb4,
        ],
    },
    OmarchyTheme {
        name: "tokyo-night",
        background: 0x1a1b26,
        foreground: 0xa9b1d6,
        palette: [
            0x1a1b26, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xad8ee6, 0x449dab, 0xa9b1d6,
            0x414868, 0xff7a93, 0xb9f27c, 0xff9e64, 0x7da6ff, 0xbb9af7, 0x0db9d7, 0xc0caf5,
        ],
    },
    OmarchyTheme {
        name: "vantablack",
        background: 0x000000,
        foreground: 0xffffff,
        palette: [
            0x000000, 0xa4a4a4, 0xb6b6b6, 0xcecece, 0x8d8d8d, 0x9b9b9b, 0xb0b0b0, 0xffffff,
            0x7a7a7a, 0xa4a4a4, 0xb6b6b6, 0xcecece, 0x8d8d8d, 0x9b9b9b, 0xb0b0b0, 0xffffff,
        ],
    },
    OmarchyTheme {
        name: "white",
        background: 0xffffff,
        foreground: 0x000000,
        palette: [
            0xffffff, 0x2a2a2a, 0x3a3a3a, 0x4a4a4a, 0x1a1a1a, 0x2e2e2e, 0x3e3e3e, 0x000000,
            0x808080, 0x2a2a2a, 0x3a3a3a, 0x4a4a4a, 0x1a1a1a, 0x2e2e2e, 0x3e3e3e, 0x000000,
        ],
    },
];
