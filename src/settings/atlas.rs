//! Atlas globe rows of the Playback tab (radio-only). One wrapper field keeps the exhaustive
//! `Field` matches to a single arm each; every Atlas-specific rule lives here.

use crate::config::AtlasConfig;
use crate::t;

use super::FieldKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AtlasField {
    Renderer,
    StationLimit,
    Panel,
    Coast,
    Grid,
    FollowPlaying,
    Autorotate,
}

impl AtlasField {
    pub const ALL: [AtlasField; 7] = [
        AtlasField::Renderer,
        AtlasField::StationLimit,
        AtlasField::Panel,
        AtlasField::Coast,
        AtlasField::Grid,
        AtlasField::FollowPlaying,
        AtlasField::Autorotate,
    ];

    pub fn kind(self) -> FieldKind {
        match self {
            AtlasField::Renderer | AtlasField::Panel | AtlasField::StationLimit => {
                FieldKind::Select
            }
            AtlasField::Coast
            | AtlasField::Grid
            | AtlasField::FollowPlaying
            | AtlasField::Autorotate => FieldKind::Toggle,
        }
    }

    pub fn label(self) -> String {
        match self {
            AtlasField::Renderer => t!("Atlas renderer", "아틀라스 렌더러", "アトラス描画"),
            AtlasField::StationLimit => t!("Atlas stations", "아틀라스 방송국 수", "アトラス局数"),
            AtlasField::Panel => t!("Atlas panel", "아틀라스 패널", "アトラスパネル"),
            AtlasField::Coast => t!("Atlas coasting", "아틀라스 관성 회전", "アトラス慣性"),
            AtlasField::Grid => t!("Atlas graticule", "아틀라스 경위선", "アトラス経緯線"),
            AtlasField::FollowPlaying => t!(
                "Atlas follows play",
                "아틀라스 재생 추적",
                "アトラス再生追従"
            ),
            AtlasField::Autorotate => {
                t!("Atlas autorotate", "아틀라스 자동 회전", "アトラス自動回転")
            }
        }
        .to_owned()
    }

    pub fn value_display(self, cfg: &AtlasConfig) -> String {
        let toggle = super::toggle_str;
        match self {
            AtlasField::Renderer => cfg.renderer.label().to_owned(),
            AtlasField::StationLimit => cfg.effective_station_limit().to_string(),
            AtlasField::Panel => cfg.panel.label().to_owned(),
            AtlasField::Coast => toggle(cfg.coast),
            AtlasField::Grid => toggle(cfg.grid),
            AtlasField::FollowPlaying => toggle(cfg.follow_playing),
            AtlasField::Autorotate => toggle(cfg.autorotate),
        }
    }

    /// Left/right (or Enter on a toggle) applied to the draft.
    pub fn step(self, cfg: &mut AtlasConfig, dir: i32) {
        let forward = dir >= 0;
        match self {
            AtlasField::Renderer => cfg.renderer = cfg.renderer.cycled(forward),
            AtlasField::StationLimit => cfg.step_station_limit(forward),
            AtlasField::Panel => cfg.panel = cfg.panel.cycled(forward),
            AtlasField::Coast => cfg.coast = !cfg.coast,
            AtlasField::Grid => cfg.grid = !cfg.grid,
            AtlasField::FollowPlaying => cfg.follow_playing = !cfg.follow_playing,
            AtlasField::Autorotate => cfg.autorotate = !cfg.autorotate,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_field_steps_and_displays() {
        let mut cfg = AtlasConfig::default();
        for field in AtlasField::ALL {
            let before = field.value_display(&cfg);
            field.step(&mut cfg, 1);
            assert_ne!(
                before,
                field.value_display(&cfg),
                "{field:?} did not change"
            );
            assert!(!field.label().is_empty());
        }
    }
}
