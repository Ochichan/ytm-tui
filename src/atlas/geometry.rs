use crate::atlas::{LatLon, wrap_longitude};
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug)]
pub struct BBox {
    pub min: LatLon,
    pub max: LatLon,
}

impl BBox {
    pub fn contains(&self, at: LatLon) -> bool {
        at.lat >= self.min.lat
            && at.lat <= self.max.lat
            && at.lon >= self.min.lon
            && at.lon <= self.max.lon
    }
}

#[derive(Debug)]
pub struct Ring {
    pub pts: Box<[LatLon]>,
    pub bbox: BBox,
    /// Absolute shoelace area in deg² over unwrapped longitudes.
    pub area: f32,
}

#[derive(Debug)]
pub struct Country {
    pub code: [u8; 2],
    pub name: Box<str>,
    pub centroid: LatLon,
    pub bbox: BBox,
    pub rings: Vec<Ring>,
}

#[derive(Debug)]
pub struct World {
    countries: Vec<Country>,
    by_code: HashMap<[u8; 2], u16>,
}

#[derive(Debug)]
pub struct DecodeError(&'static str);

impl core::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for DecodeError {}

pub fn world() -> &'static World {
    static WORLD: OnceLock<World> = OnceLock::new();
    WORLD.get_or_init(|| {
        decode(include_bytes!("../../assets/atlas/world-110m.bin"))
            .expect("embedded atlas geometry asset is corrupt")
    })
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn u8(&mut self) -> u8 {
        let v = self.bytes[self.pos];
        self.pos += 1;
        v
    }

    fn u16(&mut self) -> u16 {
        u16::from_le_bytes([self.u8(), self.u8()])
    }

    fn i16(&mut self) -> i16 {
        i16::from_le_bytes([self.u8(), self.u8()])
    }

    fn take(&mut self, n: usize) -> &'a [u8] {
        let v = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        v
    }

    fn varint(&mut self) -> u32 {
        let mut value = 0u32;
        let mut shift = 0;
        loop {
            let byte = self.u8();
            value |= u32::from(byte & 0x7F) << shift;
            if byte & 0x80 == 0 {
                return value;
            }
            shift += 7;
        }
    }
}

fn zigzag_decode(value: u32) -> i32 {
    ((value >> 1) as i32) ^ -((value & 1) as i32)
}

/// Even-odd point-in-ring; `(yj - yi)` is floored at 1e-12 as in the reference JS.
pub fn point_in_ring(pts: &[LatLon], at: LatLon) -> bool {
    let mut inside = false;
    for i in 0..pts.len() {
        let (x0, y0) = (pts[i].lon, pts[i].lat);
        let (x1, y1) = (pts[(i + 1) % pts.len()].lon, pts[(i + 1) % pts.len()].lat);
        let denom = y1 - y0;
        let denom = if denom.abs() < 1e-12 {
            if denom < 0.0 { -1e-12 } else { 1e-12 }
        } else {
            denom
        };
        if (y0 > at.lat) != (y1 > at.lat) {
            let x_cross = x0 + (at.lat - y0) / denom * (x1 - x0);
            if at.lon < x_cross {
                inside = !inside;
            }
        }
    }
    inside
}

fn signed_area(points: &[LatLon]) -> f32 {
    let mut sum = 0.0;
    let mut prev = points[0];
    for &pt in &points[1..] {
        sum += prev.lon * pt.lat - pt.lon * prev.lat;
        prev = pt;
    }
    sum += prev.lon * points[0].lat - points[0].lon * prev.lat;
    sum / 2.0
}

fn unwrap_lons(pts: &[LatLon]) -> Vec<LatLon> {
    let mut out = Vec::with_capacity(pts.len());
    out.push(pts[0]);
    for &pt in &pts[1..] {
        let prev = *out.last().expect("seeded");
        let mut lon = pt.lon;
        while lon - prev.lon > 180.0 {
            lon -= 360.0;
        }
        while lon - prev.lon < -180.0 {
            lon += 360.0;
        }
        out.push(LatLon::new(pt.lat, lon));
    }
    out
}

fn build_ring(pts: &[LatLon]) -> Ring {
    let bbox = BBox {
        min: LatLon::new(
            pts.iter().map(|p| p.lat).fold(f32::INFINITY, f32::min),
            pts.iter().map(|p| p.lon).fold(f32::INFINITY, f32::min),
        ),
        max: LatLon::new(
            pts.iter().map(|p| p.lat).fold(f32::NEG_INFINITY, f32::max),
            pts.iter().map(|p| p.lon).fold(f32::NEG_INFINITY, f32::max),
        ),
    };
    let area = signed_area(&unwrap_lons(pts)).abs();
    Ring {
        pts: pts.into(),
        bbox,
        area,
    }
}

impl Country {
    pub fn largest_ring(&self) -> Option<&Ring> {
        self.rings.iter().max_by(|a, b| a.area.total_cmp(&b.area))
    }

    /// Deterministic FNV-1a/LCG random point inside the largest ring; centroid fallback.
    pub fn estimated_location(&self, seed: &str) -> LatLon {
        let Some(ring) = self.largest_ring() else {
            return self.centroid;
        };
        let unwrapped = unwrap_lons(&ring.pts);
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        for &p in &unwrapped {
            min = min.min(p.lon);
            max = max.max(p.lon);
        }
        let mut h = 2166136261u32;
        for b in seed.as_bytes() {
            h ^= u32::from(*b);
            h = h.wrapping_mul(16777619);
        }
        for _ in 0..96 {
            h = h.wrapping_mul(1664525).wrapping_add(1013904223);
            let lon = h as f32 / 4294967296.0;
            h = h.wrapping_mul(1664525).wrapping_add(1013904223);
            let lat = h as f32 / 4294967296.0;
            let candidate = LatLon::new(
                ring.bbox.min.lat + lat * (ring.bbox.max.lat - ring.bbox.min.lat),
                min + lon * (max - min),
            );
            if point_in_ring(&unwrapped, candidate) {
                return LatLon::new(candidate.lat, wrap_longitude(candidate.lon));
            }
        }
        self.centroid
    }
}

impl World {
    pub fn countries(&self) -> &[Country] {
        &self.countries
    }

    pub fn by_code(&self, code: &str) -> Option<&Country> {
        if code.len() != 2 || !code.is_ascii() {
            return None;
        }
        let key = [
            code.as_bytes()[0].to_ascii_uppercase(),
            code.as_bytes()[1].to_ascii_uppercase(),
        ];
        self.by_code
            .get(&key)
            .and_then(|&i| self.countries.get(usize::from(i)))
    }

    /// BBox reject first, then even-odd over every stored (outer) ring.
    pub fn country_at(&self, at: LatLon) -> Option<&Country> {
        self.countries.iter().find(|c| {
            c.bbox.contains(at)
                && c.rings
                    .iter()
                    .any(|r| r.bbox.contains(at) && point_in_ring(&r.pts, at))
        })
    }
}

pub fn decode(bytes: &[u8]) -> Result<World, DecodeError> {
    let short = "asset shorter than header";
    if bytes.len() < 7 {
        return Err(DecodeError(short));
    }
    if &bytes[..4] != b"YTTA" {
        return Err(DecodeError("bad magic"));
    }
    if bytes[4] != 1 {
        return Err(DecodeError("unsupported version"));
    }
    let mut r = Reader { bytes, pos: 5 };
    let count = r.u16();
    let mut countries = Vec::with_capacity(usize::from(count));
    let mut by_code = HashMap::with_capacity(usize::from(count));
    for index in 0..count {
        let code = [r.u8(), r.u8()];
        let name_len = usize::from(r.u8());
        let name = String::from_utf8_lossy(r.take(name_len)).into_owned();
        let clat = r.i16();
        let clon = r.i16();
        let (min_lon, min_lat, max_lon, max_lat) = (r.i16(), r.i16(), r.i16(), r.i16());
        let bbox = BBox {
            min: LatLon::new(f32::from(min_lat) / 100.0, f32::from(min_lon) / 100.0),
            max: LatLon::new(f32::from(max_lat) / 100.0, f32::from(max_lon) / 100.0),
        };
        let ring_count = r.u16();
        let mut rings = Vec::with_capacity(usize::from(ring_count));
        for _ in 0..ring_count {
            let pt_count = usize::from(r.u16());
            let mut pts = Vec::with_capacity(pt_count);
            let mut lon = i32::from(r.i16());
            let mut lat = i32::from(r.i16());
            pts.push(LatLon::new(lat as f32 / 100.0, lon as f32 / 100.0));
            for _ in 1..pt_count {
                lon += zigzag_decode(r.varint());
                lat += zigzag_decode(r.varint());
                pts.push(LatLon::new(lat as f32 / 100.0, lon as f32 / 100.0));
            }
            rings.push(build_ring(&pts));
        }
        let country = Country {
            code,
            name: name.into(),
            centroid: LatLon::new(f32::from(clat) / 100.0, f32::from(clon) / 100.0),
            bbox,
            rings,
        };
        by_code.insert(country.code, index);
        countries.push(country);
    }
    if r.pos != bytes.len() {
        return Err(DecodeError("trailing bytes"));
    }
    Ok(World { countries, by_code })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fnv_pairs(world: &World) -> (usize, u32) {
        let mut hash = 2166136261u32;
        let mut total = 0;
        for country in &world.countries {
            for ring in &country.rings {
                for pt in &ring.pts {
                    let li = (pt.lon * 100.0).round() as i16;
                    let la = (pt.lat * 100.0).round() as i16;
                    for v in [li as u16, la as u16] {
                        hash = (hash ^ u32::from(v & 0xff)).wrapping_mul(16777619);
                        hash = (hash ^ u32::from(v >> 8)).wrapping_mul(16777619);
                    }
                    total += 1;
                }
            }
        }
        (total, hash)
    }

    #[test]
    fn embedded_asset_pinned() {
        let world = world();
        assert_eq!(world.countries.len(), 177);
        let (total, hash) = fnv_pairs(world);
        assert_eq!(total, 10582);
        assert_eq!(format!("{hash:08x}"), "b5083447");
    }

    #[test]
    fn country_hit_tests() {
        let world = world();
        let name = |c: Option<&Country>| c.map(|c| c.code).unwrap();
        assert_eq!(name(world.country_at(LatLon::new(37.5, 127.0))), *b"KR");
        assert_eq!(name(world.country_at(LatLon::new(72.0, -40.0))), *b"GL");
        assert!(world.country_at(LatLon::new(0.0, 0.0)).is_none());
        assert_eq!(name(world.country_at(LatLon::new(-16.3, 179.9))), *b"FJ");
        assert_eq!(name(world.country_at(LatLon::new(-16.3, -179.9))), *b"FJ");
    }

    #[test]
    fn centroid_and_estimation() {
        let world = world();
        let kr = world.by_code("kr").expect("KR");
        assert!((kr.centroid.lat - 36.0).abs() < 1.5);
        assert!((wrap_longitude(kr.centroid.lon) - 127.8).abs() < 1.5);
        let first = kr.estimated_location("abc");
        assert_eq!(first, kr.estimated_location("abc"));
        let ring = kr.largest_ring().expect("ring");
        let unwrapped = unwrap_lons(&ring.pts);
        assert!(point_in_ring(&unwrapped, first));
    }

    #[test]
    fn square_ring_edge_cases() {
        let square = [
            LatLon::new(0.0, 0.0),
            LatLon::new(0.0, 10.0),
            LatLon::new(10.0, 10.0),
            LatLon::new(10.0, 0.0),
        ];
        assert!(point_in_ring(&square, LatLon::new(5.0, 5.0)));
        assert!(!point_in_ring(&square, LatLon::new(15.0, 5.0)));
        assert!(!point_in_ring(&square, LatLon::new(-0.1, 5.0)));
        assert!(point_in_ring(&square, LatLon::new(0.0, 5.0)));
    }
}
