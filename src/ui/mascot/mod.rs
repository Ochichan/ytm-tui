pub mod asset;
pub mod generated;
pub mod render;

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::App;

/// Draw the DJ Gem start-screen mascot inside `bounds` (the strip from the reserved top band
/// down to the transcript's bottom edge, full view width). The left `TEXT_W` columns stay
/// clear for the onboarding text; the largest Momoring size that fits the rest is used, so
/// the mascot never spills over the input box or the docked player.
pub fn render_dj_gem(frame: &mut Frame, app: &App, bounds: Rect) {
    // Widest onboarding headline (JA/EN, 55 cells) plus the transcript's 2-cell left pad.
    const TEXT_W: u16 = 57;

    let max_w = bounds.width.saturating_sub(TEXT_W);
    let max_h = bounds.height;
    let asset = if app.ai.thinking {
        &generated::dj_gem::DJ_GEM_THINKING
    } else {
        let Some((idle, working)) = generated::momoring::LADDER
            .iter()
            .find(|(idle, _)| idle.width <= max_w && idle.height <= max_h)
        else {
            return;
        };
        if app.ai_mascot_active() {
            *working
        } else {
            *idle
        }
    };
    if asset.width > max_w || asset.height > max_h {
        return;
    }

    let free_w = bounds.width - asset.width;
    let area = Rect {
        x: bounds.x + (free_w * 3 / 4).max(TEXT_W),
        y: bounds.y,
        width: asset.width,
        height: asset.height,
    };
    render::render(frame, app, area, asset);
}

#[cfg(test)]
mod tests {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

    use super::generated;

    fn changed_cells(a: &[&str], b: &[&str]) -> usize {
        a.iter()
            .zip(b.iter())
            .map(|(a, b)| a.chars().zip(b.chars()).filter(|(a, b)| a != b).count())
            .sum()
    }

    fn nonblank_bounds(lines: &[&str]) -> Option<(usize, usize, usize, usize)> {
        let mut min_x = usize::MAX;
        let mut min_y = usize::MAX;
        let mut max_x = 0;
        let mut max_y = 0;
        let mut any = false;
        for (y, line) in lines.iter().enumerate() {
            for (x, ch) in line.chars().enumerate() {
                if ch == ' ' {
                    continue;
                }
                any = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
        any.then_some((min_x, min_y, max_x, max_y))
    }

    fn frame_contains(lines: &[&str], needle: &str) -> bool {
        lines.iter().any(|line| line.contains(needle))
    }

    #[test]
    fn asset_lines_match_dimensions() {
        for asset in generated::all_assets() {
            for frame in asset.frames {
                assert_eq!(
                    frame.lines.len(),
                    usize::from(asset.height),
                    "{}",
                    asset.name
                );
                for line in frame.lines {
                    assert_eq!(
                        UnicodeWidthStr::width(*line),
                        usize::from(asset.width),
                        "{} line {line:?}",
                        asset.name
                    );
                }
            }
        }
    }

    #[test]
    fn asset_glyphs_are_single_width() {
        for asset in generated::all_assets() {
            for frame in asset.frames {
                for line in frame.lines {
                    for ch in line.chars() {
                        assert_eq!(
                            UnicodeWidthChar::width(ch),
                            Some(1),
                            "{} {ch:?}",
                            asset.name
                        );
                    }
                }
            }
        }
    }

    // Momoring's working strip is deliberately outside this budget: edge-braille of a
    // moving pixel sprite flips ~50% of cells per frame at every size, which is the motion
    // itself, not flicker. It is covered by `momoring_ladder_is_uniform` instead.
    const GROOVE_ASSETS: [&crate::ui::mascot::asset::MascotAsset; 2] = [
        &generated::dj_gem::DJ_GEM_GROOVE,
        &generated::cat_laptop::CAT_LAPTOP_GROOVE,
    ];

    #[test]
    fn groove_animation_stays_under_cell_change_budget() {
        for asset in GROOVE_ASSETS {
            let total_cells = usize::from(asset.width) * usize::from(asset.height);
            let max_changed = total_cells * 15 / 100;
            for pair in asset.frames.windows(2) {
                let changed = changed_cells(pair[0].lines, pair[1].lines);
                assert!(
                    changed <= max_changed,
                    "{} changed {changed}/{total_cells} cells",
                    asset.name
                );
            }

            let changed = changed_cells(
                asset.frames.last().unwrap().lines,
                asset.frames.first().unwrap().lines,
            );
            assert!(
                changed <= max_changed,
                "{} loop seam changed {changed}/{total_cells} cells",
                asset.name
            );
        }
    }

    #[test]
    fn groove_loop_returns_to_rest_pose_without_a_large_seam() {
        for asset in GROOVE_ASSETS {
            let total_cells = usize::from(asset.width) * usize::from(asset.height);
            let changed = changed_cells(
                asset.frames.last().unwrap().lines,
                asset.frames.first().unwrap().lines,
            );
            assert!(
                changed * 100 <= total_cells * 6,
                "{} loop seam changed {changed}/{total_cells} cells",
                asset.name
            );
            assert_eq!(
                nonblank_bounds(asset.frames.last().unwrap().lines),
                nonblank_bounds(asset.frames.first().unwrap().lines),
                "{} loop seam bounds should not jump",
                asset.name
            );
        }
    }

    #[test]
    fn groove_animation_keeps_nonblank_bounds_stable() {
        for asset in GROOVE_ASSETS {
            let expected = nonblank_bounds(asset.frames[0].lines).unwrap();
            for frame in asset.frames {
                assert_eq!(
                    nonblank_bounds(frame.lines),
                    Some(expected),
                    "{} frame bounds should not jump by a cell",
                    asset.name
                );
            }
        }
    }

    #[test]
    fn dj_gem_frames_keep_24x15_silhouette_and_core_features() {
        for asset in [
            &generated::dj_gem::DJ_GEM_IDLE,
            &generated::dj_gem::DJ_GEM_GROOVE,
            &generated::dj_gem::DJ_GEM_THINKING,
            &generated::dj_gem::DJ_GEM_IDLE_RETRO,
            &generated::dj_gem::DJ_GEM_GROOVE_RETRO,
            &generated::dj_gem::DJ_GEM_THINKING_RETRO,
        ] {
            assert!(asset.width <= 24, "{}", asset.name);
            assert!(asset.height <= 15, "{}", asset.name);
            for frame in asset.frames {
                let (min_x, min_y, max_x, max_y) = nonblank_bounds(frame.lines).unwrap();
                assert!(
                    max_x - min_x + 1 >= 18,
                    "{} silhouette too narrow",
                    asset.name
                );
                assert!(
                    max_y - min_y + 1 >= 14,
                    "{} silhouette too short",
                    asset.name
                );
                assert!(frame_contains(frame.lines, "/\\"), "{} ears", asset.name);
                assert!(frame_contains(frame.lines, "DJ"), "{} label DJ", asset.name);
                assert!(
                    frame_contains(frame.lines, "GEM"),
                    "{} label GEM",
                    asset.name
                );
                assert!(
                    frame_contains(frame.lines, "||"),
                    "{} body/legs",
                    asset.name
                );
                assert!(
                    frame_contains(frame.lines, "\\____/") || frame_contains(frame.lines, "___"),
                    "{} mouth detail",
                    asset.name
                );
            }
        }
    }

    #[test]
    fn momoring_ladder_is_uniform() {
        // `animation_draw_fps` reads the largest entry's fps for the whole ladder, and the
        // idle asset must be the working asset's rest pose so the pair swaps without a jump.
        let ladder = &generated::momoring::LADDER;
        let fps = ladder[0].1.fps;
        for (idle, working) in ladder.iter() {
            assert_eq!(working.fps, fps, "{}", working.name);
            assert_eq!(idle.fps, fps, "{}", idle.name);
            assert_eq!(idle.frames.len(), 1, "{}", idle.name);
            assert_eq!(
                (idle.width, idle.height),
                (working.width, working.height),
                "{}",
                idle.name
            );
            assert!(
                std::ptr::eq(idle.frames[0].lines, working.frames[0].lines),
                "{} should be {}'s rest pose",
                idle.name,
                working.name
            );
        }
        assert!(
            ladder
                .windows(2)
                .all(|pair| pair[0].0.height > pair[1].0.height),
            "ladder must be sorted largest first"
        );
    }

    #[test]
    fn regions_fit_within_asset_bounds() {
        for asset in generated::all_assets() {
            for region in asset.regions {
                assert!(
                    region.w > 0 && region.h > 0,
                    "{} has an empty region",
                    asset.name
                );
                assert!(
                    region.x + region.w <= asset.width && region.y + region.h <= asset.height,
                    "{} region ({}, {}) {}x{} escapes the {}x{} asset",
                    asset.name,
                    region.x,
                    region.y,
                    region.w,
                    region.h,
                    asset.width,
                    asset.height
                );
            }
        }
    }

    #[test]
    fn cat_laptop_family_shares_one_region_map() {
        // Retro fallbacks must carry the same overlays as their primary asset, so the
        // multi-color treatment survives retro mode.
        let family = [
            &generated::cat_laptop::CAT_LAPTOP_IDLE,
            &generated::cat_laptop::CAT_LAPTOP_IDLE_RETRO,
            &generated::cat_laptop::CAT_LAPTOP_GROOVE,
            &generated::cat_laptop::CAT_LAPTOP_GROOVE_RETRO,
        ];
        for asset in family {
            assert!(
                !asset.regions.is_empty(),
                "{} should carry the shared color regions",
                asset.name
            );
            assert!(
                std::ptr::eq(
                    asset.regions.as_ptr(),
                    generated::cat_laptop::CAT_LAPTOP_IDLE.regions.as_ptr()
                ),
                "{} should share the family region map",
                asset.name
            );
        }
    }

    #[test]
    fn retro_asset_is_ascii_safe() {
        let retro_assets: Vec<_> = generated::all_assets()
            .iter()
            .filter(|asset| asset.name.ends_with("_retro"))
            .collect();
        assert!(
            retro_assets.len() >= 5,
            "every asset family should register its retro variants"
        );
        for asset in retro_assets {
            for frame in asset.frames {
                for line in frame.lines {
                    for ch in line.chars() {
                        assert!(
                            ch.is_ascii() && !ch.is_ascii_control(),
                            "{} {ch:?}",
                            asset.name
                        );
                    }
                }
            }
        }
    }
}
