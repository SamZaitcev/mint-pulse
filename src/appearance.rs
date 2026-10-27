#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Auto,
    Dark,
    Light,
    HighContrast,
}

impl Theme {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "auto" => Ok(Self::Auto),
            "dark" => Ok(Self::Dark),
            "light" => Ok(Self::Light),
            "high-contrast" => Ok(Self::HighContrast),
            _ => Err("Theme must be auto, dark, light or high-contrast".into()),
        }
    }

    pub fn from_name(value: &str) -> Self {
        let name = value.to_ascii_lowercase().replace(['-', '_', ' '], "");

        if name.contains("highcontrast") {
            Self::HighContrast
        } else if name.contains("dark") {
            Self::Dark
        } else {
            Self::Light
        }
    }

    pub fn palette(self) -> Palette {
        match self {
            Self::Light => Palette {
                background: 0xe6eaed,
                text: 0x20272b,
                graphs: [0x007cad, 0x518327, 0x815bad, 0x9b751a],
                critical: 0xc82020,
                fill_alpha: 0.38,
            },
            Self::HighContrast => Palette {
                background: 0x000000,
                text: 0xffffff,
                graphs: [0x54cfff, 0xb5ed70, 0xd2b1ff, 0xffd477],
                critical: 0xff5757,
                fill_alpha: 0.55,
            },
            Self::Auto | Self::Dark => Palette {
                background: 0x151a1d,
                text: 0xdedede,
                graphs: [0x1f9ede, 0x9ac56b, 0xb49bd5, 0xd6b16b],
                critical: 0xfc4138,
                fill_alpha: 0.45,
            },
        }
    }
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub background: u32,
    pub text: u32,
    pub graphs: [u32; 4],
    pub critical: u32,
    pub fill_alpha: f64,
}
