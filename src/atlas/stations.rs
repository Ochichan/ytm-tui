//! Atlas station catalog: merge policy, search, and marker projection.

use std::collections::{HashMap, HashSet, VecDeque};

use super::{LatLon, Marker, MarkerKind, wrap_longitude};
use crate::api::Song;
use crate::api::radio_browser::RadioStation;

/// Country codes are seeded for estimated coordinates, so a missing code cannot be guessed.
pub trait LocationEstimator {
    fn estimate(&self, country_code: &str, seed: &str) -> Option<LatLon>;
}

impl<F> LocationEstimator for F
where
    F: Fn(&str, &str) -> Option<LatLon>,
{
    fn estimate(&self, country_code: &str, seed: &str) -> Option<LatLon> {
        self(country_code, seed)
    }
}

pub const CATALOG_MAX: usize = 5500;

fn code_bytes(code: &str) -> [u8; 2] {
    let upper: String = code
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .map(|c| c.to_ascii_uppercase())
        .take(2)
        .collect();
    let bytes = upper.as_bytes();
    [
        bytes.first().copied().unwrap_or(b' '),
        bytes.get(1).copied().unwrap_or(b' '),
    ]
}

/// Serde is manual here because `LatLon` intentionally stays a pure geometry type.
impl serde::Serialize for AtlasStation {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(serde::Serialize)]
        struct Row<'a> {
            uuid: &'a str,
            name: &'a str,
            url: &'a str,
            country_code: [u8; 2],
            country: &'a str,
            state: &'a str,
            language: &'a str,
            tags: &'a str,
            codec: &'a str,
            bitrate: u16,
            votes: u32,
            clicks: u32,
            pos: [f32; 2],
            estimated: bool,
        }
        Row {
            uuid: &self.uuid,
            name: &self.name,
            url: &self.url,
            country_code: self.country_code,
            country: &self.country,
            state: &self.state,
            language: &self.language,
            tags: &self.tags,
            codec: &self.codec,
            bitrate: self.bitrate,
            votes: self.votes,
            clicks: self.clicks,
            pos: [self.pos.lat, self.pos.lon],
            estimated: self.estimated,
        }
        .serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for AtlasStation {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        struct Row {
            uuid: String,
            name: String,
            url: String,
            country_code: [u8; 2],
            country: String,
            state: String,
            language: String,
            tags: String,
            codec: String,
            bitrate: u16,
            votes: u32,
            clicks: u32,
            pos: [f32; 2],
            estimated: bool,
        }
        let row = Row::deserialize(deserializer)?;
        let mut station = Self {
            uuid: row.uuid.into(),
            name: row.name.into(),
            url: row.url.into(),
            country_code: row.country_code,
            country: row.country.into(),
            state: row.state.into(),
            language: row.language.into(),
            tags: row.tags.into(),
            codec: row.codec.into(),
            bitrate: row.bitrate,
            votes: row.votes,
            clicks: row.clicks,
            pos: LatLon::new(row.pos[0], row.pos[1]),
            estimated: row.estimated,
            unit: [0.0; 3],
            search_key: Box::from(""),
        };
        station.finish();
        Ok(station)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AtlasStation {
    pub uuid: Box<str>,
    pub name: Box<str>,
    pub url: Box<str>,
    pub country_code: [u8; 2],
    pub country: Box<str>,
    pub state: Box<str>,
    pub language: Box<str>,
    pub tags: Box<str>,
    pub codec: Box<str>,
    pub bitrate: u16,
    pub votes: u32,
    pub clicks: u32,
    pub pos: LatLon,
    pub estimated: bool,
    pub unit: [f32; 3],
    /// Lower-cased `name / country / cc / state / language / tags / codec`, built once so a
    /// keystroke in the search box does not re-lowercase every field of every station.
    pub search_key: Box<str>,
}

impl AtlasStation {
    pub fn finish(&mut self) {
        self.pos.lon = wrap_longitude(self.pos.lon);
        self.unit = self.pos.to_unit();
        self.search_key = [
            self.name.as_ref(),
            self.country.as_ref(),
            std::str::from_utf8(&self.country_code).unwrap_or(""),
            self.state.as_ref(),
            self.language.as_ref(),
            self.tags.as_ref(),
            self.codec.as_ref(),
        ]
        .join("\n")
        .to_lowercase()
        .into();
    }

    pub fn from_radio(r: RadioStation, est: &dyn LocationEstimator) -> Option<Self> {
        let (pos, estimated) = if let (Some(lat), Some(lon)) = (r.lat, r.lon) {
            (LatLon::new(lat, lon), false)
        } else {
            (est.estimate(&r.country_code, &r.uuid)?, true)
        };
        Some(Self {
            uuid: r.uuid.into(),
            name: r.name.into(),
            url: r.url.into(),
            country_code: code_bytes(&r.country_code),
            country: r.country.into(),
            state: r.state.into(),
            language: r.language.into(),
            tags: r.tags.into(),
            codec: r.codec.into(),
            bitrate: r.bitrate.min(u16::MAX as u32) as u16,
            votes: r.votes,
            clicks: r.clicks,
            pos,
            estimated,
            unit: [0.0; 3],
            search_key: Box::from(""),
        })
        .map(|mut station| {
            station.finish();
            station
        })
    }

    pub fn to_radio(&self) -> RadioStation {
        RadioStation {
            uuid: self.uuid.to_string(),
            name: self.name.to_string(),
            url: self.url.to_string(),
            country: self.country.to_string(),
            country_code: String::from_utf8_lossy(&self.country_code)
                .trim()
                .to_string(),
            state: self.state.to_string(),
            language: self.language.to_string(),
            tags: self.tags.to_string(),
            codec: self.codec.to_string(),
            bitrate: u32::from(self.bitrate),
            votes: self.votes,
            clicks: self.clicks,
            lat: Some(self.pos.lat),
            lon: Some(self.pos.lon),
        }
    }

    pub fn to_song(&self) -> Song {
        self.clone().to_radio().into_song()
    }

    pub fn meta_line(&self) -> String {
        let cc = String::from_utf8_lossy(&self.country_code)
            .trim()
            .to_string();
        let bitrate = if self.bitrate > 0 {
            format!("{} kbps", self.bitrate)
        } else {
            String::new()
        };
        [cc.as_str(), self.codec.as_ref(), bitrate.as_str()]
            .into_iter()
            .filter(|s| !s.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" · ")
    }

    pub fn matches(&self, needle_lower: &str) -> bool {
        self.search_key.contains(needle_lower)
    }
}

#[derive(Debug)]
pub struct Catalog {
    stations: Vec<AtlasStation>,
    by_uuid: HashMap<Box<str>, usize>,
    pinned: HashSet<Box<str>>,
    capacity: usize,
    pub version: u64,
}

impl Default for Catalog {
    fn default() -> Self {
        Self::with_capacity(CATALOG_MAX)
    }
}

impl Catalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            stations: Vec::new(),
            by_uuid: HashMap::new(),
            pinned: HashSet::new(),
            capacity: capacity.max(1),
            version: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.stations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stations.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&AtlasStation> {
        self.stations.get(index)
    }

    pub fn index_of(&self, uuid: &str) -> Option<usize> {
        self.by_uuid.get(uuid).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = &AtlasStation> {
        self.stations.iter()
    }

    /// Indices are stable: rows are only ever replaced in place, never shifted, so an index
    /// the App holds (selection, highlight) keeps naming the same slot across merges. A
    /// country merge that needs room overwrites the oldest unpinned slot.
    fn merge(&mut self, rows: Vec<AtlasStation>, pin: bool) -> usize {
        let mut changed = 0;
        for row in rows {
            let uuid: Box<str> = row.uuid.clone();
            let index = if let Some(&index) = self.by_uuid.get(&*uuid) {
                if self.pinned.contains(&uuid) && !pin {
                    continue;
                }
                if self.stations[index] == row {
                    if pin {
                        self.pinned.insert(uuid);
                    }
                    continue;
                }
                self.stations[index] = row;
                index
            } else if self.stations.len() < self.capacity {
                self.stations.push(row);
                self.stations.len() - 1
            } else if pin {
                let Some(victim) = self
                    .stations
                    .iter()
                    .position(|s| !self.pinned.contains(&s.uuid))
                else {
                    continue;
                };
                self.by_uuid.remove(&self.stations[victim].uuid);
                self.stations[victim] = row;
                victim
            } else {
                continue;
            };
            self.by_uuid.insert(uuid.clone(), index);
            if pin {
                self.pinned.insert(uuid);
            }
            changed += 1;
        }
        if changed > 0 {
            self.version += 1;
        }
        changed
    }

    pub fn merge_world(&mut self, rows: Vec<AtlasStation>) -> usize {
        self.merge(rows, false)
    }

    pub fn merge_country(&mut self, rows: Vec<AtlasStation>) -> usize {
        self.merge(rows, true)
    }

    pub fn markers(
        &self,
        selected: Option<usize>,
        highlight: Option<usize>,
        favorites: &dyn Fn(&str) -> bool,
    ) -> Vec<Marker> {
        self.stations
            .iter()
            .enumerate()
            .map(|(index, station)| {
                let kind = if Some(index) == selected {
                    MarkerKind::Selected
                } else if Some(index) == highlight {
                    MarkerKind::Highlight
                } else if favorites(&station.uuid) {
                    MarkerKind::Favorite
                } else if station.estimated {
                    MarkerKind::Estimated
                } else {
                    MarkerKind::Normal
                };
                Marker {
                    index,
                    unit: station.unit,
                    kind,
                }
            })
            .collect()
    }

    pub fn search(&self, query: &str, max: usize) -> Vec<usize> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        self.stations
            .iter()
            .enumerate()
            .filter(|(_, s)| s.matches(&needle))
            .map(|(index, _)| index)
            .take(max)
            .collect()
    }

    pub fn by_country(&self, code: &str, max: usize) -> Vec<usize> {
        let needle = code_bytes(code);
        self.stations
            .iter()
            .enumerate()
            .filter(|(_, s)| s.country_code == needle)
            .map(|(index, _)| index)
            .take(max)
            .collect()
    }

    pub fn random_avoiding(&self, recent: &[Box<str>], rng_seed: u64) -> Option<usize> {
        if self.stations.is_empty() {
            return None;
        }
        let mut rng = rng_seed | 1;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        // A random start, then the first non-recent station scanning forward: always avoids
        // the recent set when any alternative exists, and stays deterministic for a seed.
        let len = self.stations.len();
        let start = (next() as usize) % len;
        (0..len)
            .map(|offset| (start + offset) % len)
            .find(|&index| !recent.contains(&self.stations[index].uuid))
            .or(Some(start))
    }

    /// The visible station closest to the view axis (greatest depth), preferring one other
    /// than `exclude`; falls back to `exclude` itself when it is the only visible station.
    pub fn nearest_visible(&self, cam_axis: [f32; 3], exclude: Option<usize>) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        let mut best_any: Option<(usize, f32)> = None;
        for (index, station) in self.stations.iter().enumerate() {
            let depth = station.unit[0] * cam_axis[0]
                + station.unit[1] * cam_axis[1]
                + station.unit[2] * cam_axis[2];
            if depth < 0.0 {
                continue;
            }
            if best_any.is_none_or(|(_, d)| depth > d) {
                best_any = Some((index, depth));
            }
            if Some(index) != exclude && best.is_none_or(|(_, d)| depth > d) {
                best = Some((index, depth));
            }
        }
        best.or(best_any).map(|(index, _)| index)
    }
}

/// Bounded recent-tuned ring; duplicates move a uuid back to the newest slot.
#[derive(Debug, Default)]
pub struct RecentTuned(VecDeque<Box<str>>);

impl RecentTuned {
    pub const CAP: usize = 10;

    pub fn push(&mut self, uuid: &str) {
        self.0.retain(|existing| existing.as_ref() != uuid);
        if self.0.len() > Self::CAP.saturating_sub(1) {
            self.0.pop_back();
        }
        self.0.push_front(uuid.into());
    }

    pub fn as_slice(&mut self) -> &[Box<str>] {
        self.0.make_contiguous()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn station(uuid: &str, lat: f32, lon: f32) -> AtlasStation {
        let mut s = AtlasStation {
            uuid: uuid.into(),
            name: format!("Station {uuid}").into(),
            url: format!("https://stream.example.org/{uuid}").into(),
            country_code: *b"KR",
            country: "South Korea".into(),
            state: "Seoul".into(),
            language: "korean".into(),
            tags: "kpop".into(),
            codec: "MP3".into(),
            bitrate: 128,
            votes: 1,
            clicks: 2,
            pos: LatLon::new(lat, lon),
            estimated: false,
            unit: [0.0; 3],
            search_key: Box::from(""),
        };
        s.finish();
        s
    }

    #[test]
    fn from_radio_prefers_geo_and_falls_back_to_estimator() {
        let est = |_cc: &str, seed: &str| Some(LatLon::new(10.0 + seed.len() as f32, 20.0));
        let geo = RadioStation::parse(&serde_json::json!({
            "stationuuid": "geo", "url": "https://a", "geo_lat": 1.0, "geo_long": 2.0,
        }))
        .unwrap();
        let s = AtlasStation::from_radio(geo, &est)
            .ok_or_else(|| "geo parse".to_string())
            .unwrap();
        assert_eq!(s.pos, LatLon::new(1.0, 2.0));
        assert!(!s.estimated);
        assert_eq!(s.unit, LatLon::new(1.0, 2.0).to_unit());

        let bare = RadioStation::parse(&serde_json::json!({
            "stationuuid": "bare", "url": "https://b", "countrycode": "KR",
        }))
        .unwrap();
        let s = AtlasStation::from_radio(bare, &est).unwrap();
        assert!(s.estimated);
        assert_eq!(s.country_code, *b"KR");
    }

    #[test]
    fn from_radio_drops_when_estimator_has_no_location() {
        let bare = RadioStation::parse(&serde_json::json!({
            "stationuuid": "bare", "url": "https://b",
        }))
        .unwrap();
        let never: fn(&str, &str) -> Option<LatLon> = |_, _| None;
        assert!(AtlasStation::from_radio(bare, &never).is_none());
    }

    #[test]
    fn merge_world_replaces_existing_but_not_pinned() {
        let mut catalog = Catalog::new();
        assert_eq!(catalog.merge_world(vec![station("a", 1.0, 1.0)]), 1);
        assert_eq!(catalog.merge_world(vec![station("a", 2.0, 2.0)]), 1);
        assert_eq!(catalog.get(0).unwrap().pos.lat, 2.0);
        assert_eq!(catalog.version, 2);
        // A country row takes over an unpinned world row and pins it; world rows can no
        // longer replace it afterwards.
        assert_eq!(catalog.merge_country(vec![station("a", 3.0, 3.0)]), 1);
        assert_eq!(catalog.get(0).unwrap().pos.lat, 3.0);
        assert_eq!(catalog.version, 3);
        assert_eq!(catalog.merge_world(vec![station("a", 4.0, 4.0)]), 0);
        assert_eq!(catalog.get(0).unwrap().pos.lat, 3.0);
        assert_eq!(catalog.version, 3);
    }

    #[test]
    fn merge_country_pins_and_no_change_no_version_bump() {
        let mut catalog = Catalog::new();
        assert_eq!(catalog.merge_country(vec![station("k", 1.0, 1.0)]), 1);
        let version = catalog.version;
        assert_eq!(catalog.merge_country(vec![station("k", 1.0, 1.0)]), 0);
        assert_eq!(catalog.version, version);
        assert!(catalog.pinned.contains("k"));
    }

    #[test]
    fn merge_country_evicts_oldest_unpinned_when_full() {
        let mut catalog = Catalog::with_capacity(3);
        for i in 0..3 {
            catalog.merge_world(vec![station(&format!("w{i}"), 0.0, i as f32)]);
        }
        let w1 = catalog.index_of("w1").unwrap();
        catalog.merge_country(vec![station("p", 0.0, 9.0)]);
        assert_eq!(catalog.merge_world(vec![station("x", 0.0, 10.0)]), 0);
        assert_eq!(catalog.len(), 3);
        assert!(
            catalog.index_of("w0").is_none(),
            "oldest unpinned row was evicted"
        );
        assert_eq!(
            catalog.index_of("p"),
            Some(0),
            "the evicted slot is reused in place"
        );
        assert_eq!(
            catalog.index_of("w1"),
            Some(w1),
            "other indices stay stable"
        );
        assert!(catalog.index_of("x").is_none());
    }

    #[test]
    fn catalog_enforces_max() {
        let mut catalog = Catalog::new();
        let rows = (0..(CATALOG_MAX + 5) as u32)
            .map(|i| station(&format!("u{i:05}"), 0.0, i as f32))
            .collect();
        assert_eq!(catalog.merge_world(rows), CATALOG_MAX);
        assert_eq!(catalog.len(), CATALOG_MAX);
    }

    #[test]
    fn search_matches_all_fields_case_insensitively() {
        let mut catalog = Catalog::new();
        catalog.merge_world(vec![station("s1", 0.0, 0.0)]);
        for (needle, expected) in [
            ("station", true),
            ("KOREA", true),
            ("kr", true),
            ("seoul", true),
            ("korean", true),
            ("kpop", true),
            ("mp3", true),
            ("jazz", false),
        ] {
            assert_eq!(!catalog.search(needle, 10).is_empty(), expected, "{needle}");
        }
    }

    #[test]
    fn by_country_filters_by_code() {
        let mut catalog = Catalog::new();
        let mut jp = station("jp", 0.0, 1.0);
        jp.country_code = *b"JP";
        catalog.merge_world(vec![station("kr", 0.0, 0.0), jp]);
        let hits = catalog.by_country("KR", 10);
        assert_eq!(hits, vec![0]);
    }

    #[test]
    fn random_avoiding_is_deterministic_and_respects_recent() {
        let mut catalog = Catalog::new();
        catalog.merge_world(vec![station("a", 0.0, 0.0), station("b", 0.0, 1.0)]);
        let first = catalog.random_avoiding(&[], 7).unwrap();
        assert_eq!(first, catalog.random_avoiding(&[], 7).unwrap());
        let recent: Vec<Box<str>> = vec![catalog.get(first).unwrap().uuid.clone()];
        let other = catalog.random_avoiding(&recent, 7).unwrap();
        assert_ne!(first, other);
    }

    #[test]
    fn nearest_visible_picks_deepest_and_honours_exclude() {
        let mut catalog = Catalog::new();
        catalog.merge_world(vec![station("far", 0.0, 80.0), station("near", 0.0, 10.0)]);
        let axis = LatLon::new(0.0, 0.0).to_unit();
        assert_eq!(
            catalog.nearest_visible(axis, None).unwrap(),
            1,
            "deepest = closest to centre"
        );
        assert_eq!(
            catalog.nearest_visible(axis, Some(1)).unwrap(),
            0,
            "exclude yields the runner-up"
        );
        let behind = LatLon::new(0.0, 180.0).to_unit();
        assert!(catalog.nearest_visible(behind, None).is_none());
    }

    #[test]
    fn to_song_equals_radio_into_song() {
        let mut catalog = Catalog::new();
        catalog.merge_world(vec![station("eq", 3.0, 4.0)]);
        let atlas_station = catalog.get(0).unwrap();
        assert_eq!(
            atlas_station.to_song(),
            RadioStation::parse(&serde_json::json!({
                "stationuuid": "eq",
                "name": "Station eq",
                "url": "https://stream.example.org/eq",
                "country": "South Korea",
                "countrycode": "KR",
                "state": "Seoul",
                "language": "korean",
                "tags": "kpop",
                "codec": "MP3",
                "bitrate": 128,
                "votes": 1,
                "clickcount": 2,
                "geo_lat": 3.0,
                "geo_long": 4.0,
            }))
            .unwrap()
            .into_song()
        );
    }

    #[test]
    fn meta_line_joins_non_empty_parts() {
        let mut catalog = Catalog::new();
        catalog.merge_world(vec![station("m", 0.0, 0.0)]);
        assert_eq!(catalog.get(0).unwrap().meta_line(), "KR · MP3 · 128 kbps");
    }

    #[test]
    fn markers_use_kind_priority() {
        let mut catalog = Catalog::new();
        catalog.merge_world(vec![station("a", 0.0, 0.0), station("b", 0.0, 1.0)]);
        let markers = catalog.markers(Some(0), None, &|uuid| uuid == "b");
        assert_eq!(markers[0].kind, MarkerKind::Selected);
        assert_eq!(markers[1].kind, MarkerKind::Favorite);
    }

    #[test]
    fn recent_tuned_dedupes_and_caps() {
        let mut recent = RecentTuned::default();
        for i in 0..12 {
            recent.push(&format!("u{i}"));
        }
        recent.push("u2");
        assert_eq!(recent.as_slice().len(), RecentTuned::CAP);
        assert_eq!(recent.as_slice().first().unwrap().as_ref(), "u2");
    }
}
