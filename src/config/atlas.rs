//! Atlas globe settings (dedicated Radio mode).

use serde::{Deserialize, Serialize};

use crate::t;

/// How the globe is drawn into cells. `Auto` picks Braille, or plain ASCII when retro mode
/// is on (the retro scrubber would otherwise flatten Braille into a halftone we cannot
/// control). `Ascii` exists for terminals whose font lacks U+2800.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AtlasRenderer {
    #[default]
    Auto,
    Braille,
    Ascii,
}

impl AtlasRenderer {
    pub fn cycled(self, forward: bool) -> Self {
        match (self, forward) {
            (Self::Auto, true) | (Self::Ascii, false) => Self::Braille,
            (Self::Braille, true) | (Self::Auto, false) => Self::Ascii,
            (Self::Ascii, true) | (Self::Braille, false) => Self::Auto,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => t!("Auto", "자동", "自動"),
            Self::Braille => t!("Braille dots", "점자 도트", "点字ドット"),
            Self::Ascii => t!("ASCII", "ASCII", "ASCII"),
        }
    }
}

/// Whether the station panel beside the globe starts visible. `Auto` shows it when the
/// content area is wide enough (≥ 96 columns).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AtlasPanel {
    #[default]
    Auto,
    Shown,
    Hidden,
}

impl AtlasPanel {
    pub fn cycled(self, forward: bool) -> Self {
        match (self, forward) {
            (Self::Auto, true) | (Self::Hidden, false) => Self::Shown,
            (Self::Shown, true) | (Self::Auto, false) => Self::Hidden,
            (Self::Hidden, true) | (Self::Shown, false) => Self::Auto,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => t!(
                "Auto (wide terminals)",
                "자동 (넓은 터미널)",
                "自動 (広い端末)"
            ),
            Self::Shown => t!("Shown", "표시", "表示"),
            Self::Hidden => t!("Hidden", "숨김", "非表示"),
        }
    }
}

pub const ATLAS_STATION_LIMIT_MIN: u32 = 500;
pub const ATLAS_STATION_LIMIT_MAX: u32 = 5000;
pub const ATLAS_STATION_LIMIT_STEP: u32 = 500;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AtlasConfig {
    pub renderer: AtlasRenderer,
    /// How many world stations to load progressively (500..=5000). Only network and cache
    /// cost scale with it; render cost is capped by the catalog ceiling.
    pub station_limit: u32,
    /// Flick-to-coast after a drag. Also requires the animations master.
    pub coast: bool,
    /// Draw the 30° graticule.
    pub grid: bool,
    /// Centre the globe on a station once when it starts playing.
    pub follow_playing: bool,
    /// Slow idle spin (animations master required).
    pub autorotate: bool,
    pub panel: AtlasPanel,
}

impl Default for AtlasConfig {
    fn default() -> Self {
        Self {
            renderer: AtlasRenderer::Auto,
            station_limit: 2000,
            coast: true,
            grid: true,
            follow_playing: true,
            autorotate: false,
            panel: AtlasPanel::Auto,
        }
    }
}

impl AtlasConfig {
    pub fn effective_station_limit(&self) -> u32 {
        self.station_limit
            .clamp(ATLAS_STATION_LIMIT_MIN, ATLAS_STATION_LIMIT_MAX)
    }

    pub fn step_station_limit(&mut self, forward: bool) {
        let cur = self.effective_station_limit();
        self.station_limit = if forward {
            (cur + ATLAS_STATION_LIMIT_STEP).min(ATLAS_STATION_LIMIT_MAX)
        } else {
            cur.saturating_sub(ATLAS_STATION_LIMIT_STEP)
                .max(ATLAS_STATION_LIMIT_MIN)
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_clamps() {
        let mut c = AtlasConfig::default();
        assert_eq!(c.effective_station_limit(), 2000);
        c.station_limit = 99_999;
        assert_eq!(c.effective_station_limit(), ATLAS_STATION_LIMIT_MAX);
        c.step_station_limit(true);
        assert_eq!(c.station_limit, ATLAS_STATION_LIMIT_MAX);
        c.station_limit = 0;
        c.step_station_limit(false);
        assert_eq!(c.station_limit, ATLAS_STATION_LIMIT_MIN);
    }

    #[test]
    fn enums_cycle_through_every_variant() {
        let mut r = AtlasRenderer::Auto;
        for _ in 0..3 {
            r = r.cycled(true);
        }
        assert_eq!(r, AtlasRenderer::Auto);
        let mut p = AtlasPanel::Auto;
        for _ in 0..3 {
            p = p.cycled(false);
        }
        assert_eq!(p, AtlasPanel::Auto);
    }

    #[test]
    fn unknown_fields_fall_back_to_defaults() {
        let parsed: AtlasConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(parsed, AtlasConfig::default());
    }
}
