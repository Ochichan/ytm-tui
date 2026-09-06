//! Atlas: the interactive terminal globe for dedicated Radio mode.
//!
//! Pure core with no `App` dependency. Split by concern so each piece is testable alone:
//! `geometry` (embedded Natural Earth polygons, point-in-polygon, centroids),
//! `mask` (equirectangular land raster sampled per screen dot), `view` (orthographic
//! camera, unprojection, kinetic rotation), `raster` (dot grid → Braille/ASCII cells),
//! `stations` (Radio Browser catalog, merge policy, hit-testing), `fetch` (network actor +
//! disk cache). The App integration lives in `crate::app::atlas`, rendering in
//! `crate::ui::views::atlas`.

pub mod fetch;
pub mod geometry;
pub mod mask;
pub mod raster;
pub mod stations;
pub mod view;

/// A geographic coordinate in degrees. `lon` is kept in (−180, 180].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LatLon {
    pub lat: f32,
    pub lon: f32,
}

impl LatLon {
    pub const fn new(lat: f32, lon: f32) -> Self {
        Self { lat, lon }
    }

    /// Unit vector on the sphere: x toward lon 0 on the equator, z toward the north pole.
    /// Every module projects through this one definition so world vectors never disagree.
    pub fn to_unit(self) -> [f32; 3] {
        let (lat, lon) = (self.lat.to_radians(), self.lon.to_radians());
        let c = lat.cos();
        [c * lon.cos(), c * lon.sin(), lat.sin()]
    }
}

/// Wrap a longitude in degrees into (−180, 180].
pub fn wrap_longitude(lon: f32) -> f32 {
    let mut w = lon % 360.0;
    if w > 180.0 {
        w -= 360.0;
    } else if w <= -180.0 {
        w += 360.0;
    }
    w
}

/// Something that answers "is there land here?" for a coordinate. The land mask
/// implements it; tests use closures/stubs so the raster never depends on real geometry.
pub trait LandLookup {
    fn is_land(&self, at: LatLon) -> bool;
}

impl<F: Fn(LatLon) -> bool> LandLookup for F {
    fn is_land(&self, at: LatLon) -> bool {
        self(at)
    }
}

/// What a station marker means visually. Priority when several share a cell:
/// `Selected > Highlight > Favorite > Normal > Estimated`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MarkerKind {
    Estimated,
    Normal,
    Favorite,
    Highlight,
    Selected,
}

/// A station as the renderer sees it: a precomputed unit vector plus its visual kind.
/// `index` points back into the catalog so hit-tests can name the station.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Marker {
    pub index: usize,
    pub unit: [f32; 3],
    pub kind: MarkerKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_longitude_keeps_half_open_range() {
        assert_eq!(wrap_longitude(180.0), 180.0);
        assert_eq!(wrap_longitude(-180.0), 180.0);
        assert_eq!(wrap_longitude(190.0), -170.0);
        assert_eq!(wrap_longitude(-190.0), 170.0);
        assert_eq!(wrap_longitude(0.0), 0.0);
    }

    #[test]
    fn unit_vector_axes() {
        let v = LatLon::new(0.0, 0.0).to_unit();
        assert!((v[0] - 1.0).abs() < 1e-6 && v[1].abs() < 1e-6 && v[2].abs() < 1e-6);
        let n = LatLon::new(90.0, 0.0).to_unit();
        assert!((n[2] - 1.0).abs() < 1e-6);
        let e = LatLon::new(0.0, 90.0).to_unit();
        assert!((e[1] - 1.0).abs() < 1e-6);
    }
}
