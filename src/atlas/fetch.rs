//! Atlas fetch actor: mirror requests, disk cache, and per-kind coalescing.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::api::radio_browser::{self, RadioQuery, RadioStation, USER_AGENT};
use crate::util::backpressure::{QueuePolicy, bounded_channel};
use crate::util::delivery::{DeliveryError, DeliveryReceipt, DeliveryResult};
use crate::util::safe_fs;

pub const ATLAS_JSON_MAX: usize = 4 * 1024 * 1024;
pub const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
pub const MAX_CACHED_ROWS: usize = 5500;
const REQUEST_DEADLINE: Duration = Duration::from_secs(60);

const WAKE_QUEUE: QueuePolicy = QueuePolicy::CoalescedByKey {
    name: "atlas",
    capacity: 1,
};

#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub enum AtlasErrorKind {
    Network,
    InvalidData,
    Cache,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AtlasCmd {
    World {
        generation: u64,
        limit: u32,
    },
    More {
        generation: u64,
        page: u32,
        limit: u32,
    },
    Country {
        generation: u64,
        code: String,
    },
    Search {
        generation: u64,
        query: String,
    },
    Resolve {
        generation: u64,
        uuids: Vec<String>,
    },
    Click {
        uuid: String,
    },
}

#[derive(Debug)]
pub enum AtlasEvent {
    World {
        generation: u64,
        page: u32,
        rows: Vec<RadioStation>,
        from_cache: bool,
        /// A cached page that a network refresh will follow; the reducer waits for that
        /// refresh before asking for more pages.
        refreshing: bool,
    },
    Country {
        generation: u64,
        code: String,
        rows: Vec<RadioStation>,
        from_cache: bool,
    },
    Search {
        generation: u64,
        query: String,
        rows: Vec<RadioStation>,
    },
    Resolved {
        generation: u64,
        rows: Vec<RadioStation>,
    },
    Error {
        generation: u64,
        kind: AtlasErrorKind,
        message: String,
    },
}

/// Bounded click backlog: a burst of tunes keeps the newest few pings, never grows.
const CLICK_BACKLOG: usize = 8;

/// One pending slot per command kind (latest wins), plus the click backlog. Interactive
/// kinds drain before the bulk world pages so a country browse never waits behind a 500-row
/// fetch that was already queued.
#[derive(Debug, Default)]
struct PendingSet {
    world: Option<AtlasCmd>,
    more: Option<AtlasCmd>,
    country: Option<AtlasCmd>,
    search: Option<AtlasCmd>,
    resolve: Option<AtlasCmd>,
    clicks: std::collections::VecDeque<String>,
}

impl PendingSet {
    /// Stores `cmd`; returns whether an older command of the same kind was replaced.
    fn put(&mut self, cmd: AtlasCmd) -> bool {
        let slot = match cmd {
            AtlasCmd::Click { uuid } => {
                if self.clicks.len() >= CLICK_BACKLOG {
                    self.clicks.pop_front();
                }
                self.clicks.push_back(uuid);
                return false;
            }
            AtlasCmd::World { .. } => &mut self.world,
            AtlasCmd::More { .. } => &mut self.more,
            AtlasCmd::Country { .. } => &mut self.country,
            AtlasCmd::Search { .. } => &mut self.search,
            AtlasCmd::Resolve { .. } => &mut self.resolve,
        };
        slot.replace(cmd).is_some()
    }

    fn take_next(&mut self) -> Option<AtlasCmd> {
        self.country
            .take()
            .or_else(|| self.search.take())
            .or_else(|| self.resolve.take())
            .or_else(|| self.world.take())
            .or_else(|| self.more.take())
    }

    fn take_clicks(&mut self) -> Vec<String> {
        self.clicks.drain(..).collect()
    }
}

pub struct AtlasHandle {
    wake_tx: mpsc::Sender<()>,
    latest: Arc<Mutex<PendingSet>>,
}

impl AtlasHandle {
    pub fn send(&self, cmd: AtlasCmd) -> DeliveryResult {
        let mut latest = self
            .latest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let replaced_existing = latest.put(cmd);
        match self.wake_tx.try_send(()) {
            // A full wake queue already guarantees the actor will inspect the set.
            Ok(()) | Err(mpsc::error::TrySendError::Full(())) if replaced_existing => {
                Ok(DeliveryReceipt::Coalesced {
                    replaced_existing: true,
                    evicted_oldest: false,
                })
            }
            Ok(()) | Err(mpsc::error::TrySendError::Full(())) => Ok(DeliveryReceipt::Enqueued),
            Err(mpsc::error::TrySendError::Closed(())) => {
                *latest = PendingSet::default();
                Err(DeliveryError::Closed)
            }
        }
    }
}

pub fn spawn<F>(emit: F) -> AtlasHandle
where
    F: Fn(AtlasEvent) + Send + Sync + 'static,
{
    let (wake_tx, wake_rx) = bounded_channel(WAKE_QUEUE);
    let latest = Arc::new(Mutex::new(PendingSet::default()));
    let handle = AtlasHandle {
        wake_tx,
        latest: Arc::clone(&latest),
    };
    tokio::spawn(run_actor(wake_rx, latest, emit));
    handle
}

async fn run_actor<F>(mut wake_rx: mpsc::Receiver<()>, latest: Arc<Mutex<PendingSet>>, emit: F)
where
    F: Fn(AtlasEvent) + Send + Sync + 'static,
{
    let fast_client = atlas_client(Duration::from_secs(12));
    let slow_client = atlas_client(Duration::from_secs(45));

    while wake_rx.recv().await.is_some() {
        loop {
            let (cmd, clicks) = {
                let mut set = latest
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                (set.take_next(), set.take_clicks())
            };
            // Clicks are fire-and-forget: each runs on its own task so a slow mirror never
            // delays the listing work behind it.
            for uuid in clicks {
                let client = fast_client.clone();
                tokio::spawn(async move {
                    if let Err(error) = request_click(&client, &uuid).await {
                        tracing::debug!(%uuid, error = %format!("{error:#}"), "radio click failed");
                    }
                });
            }
            let Some(cmd) = cmd else { break };
            handle_cmd(cmd, &fast_client, &slow_client, &emit).await;
        }
    }
}

fn atlas_client(timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(timeout)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

async fn handle_cmd<F>(cmd: AtlasCmd, fast: &reqwest::Client, slow: &reqwest::Client, emit: &F)
where
    F: Fn(AtlasEvent) + Send + Sync + 'static,
{
    match cmd {
        AtlasCmd::World { generation, limit } => {
            let kind = AtlasErrorKind::Network;
            match cache::load(&cache::CacheKind::World) {
                Ok(Some(loaded)) => {
                    let rows = loaded.rows;
                    let fresh = loaded.fresh;
                    emit(AtlasEvent::World {
                        generation,
                        page: 0,
                        rows: rows.clone(),
                        from_cache: true,
                        refreshing: !fresh,
                    });
                    if fresh {
                        return;
                    }
                }
                Ok(None) => {}
                Err(error) => emit(AtlasEvent::Error {
                    generation,
                    kind: AtlasErrorKind::Cache,
                    message: format!("{error:#}"),
                }),
            }
            match request_bounded(slow, &RadioQuery::World { limit }).await {
                Ok(json) => {
                    let rows = radio_browser::parse_station_list(&json, MAX_CACHED_ROWS);
                    store_rows(&cache::CacheKind::World, &rows);
                    emit(AtlasEvent::World {
                        generation,
                        page: 0,
                        rows,
                        from_cache: false,
                        refreshing: false,
                    });
                }
                Err(error) => emit(AtlasEvent::Error {
                    generation,
                    kind,
                    message: format!("{error:#}"),
                }),
            }
        }
        AtlasCmd::More {
            generation,
            page,
            limit,
        } => match request_bounded(slow, &RadioQuery::WorldMore { limit }).await {
            Ok(json) => {
                emit(AtlasEvent::World {
                    generation,
                    page,
                    rows: radio_browser::parse_station_list(&json, MAX_CACHED_ROWS),
                    from_cache: false,
                    refreshing: false,
                });
            }
            Err(error) => emit(AtlasEvent::Error {
                generation,
                kind: AtlasErrorKind::Network,
                message: format!("{error:#}"),
            }),
        },
        AtlasCmd::Country { generation, code } => {
            let Some(query) = RadioQuery::country(&code, 500) else {
                emit(AtlasEvent::Error {
                    generation,
                    kind: AtlasErrorKind::InvalidData,
                    message: format!("invalid country code {code:?}"),
                });
                return;
            };
            match cache::load(&cache::CacheKind::Country(&code)) {
                Ok(Some(loaded)) => {
                    let rows = loaded.rows;
                    let fresh = loaded.fresh;
                    emit(AtlasEvent::Country {
                        generation,
                        code: code.clone(),
                        rows: rows.clone(),
                        from_cache: true,
                    });
                    if fresh {
                        return;
                    }
                }
                Ok(None) => {}
                Err(error) => emit(AtlasEvent::Error {
                    generation,
                    kind: AtlasErrorKind::Cache,
                    message: format!("{error:#}"),
                }),
            }
            match request_bounded(slow, &query).await {
                Ok(json) => {
                    let rows = radio_browser::parse_station_list(&json, MAX_CACHED_ROWS);
                    store_rows(&cache::CacheKind::Country(&code), &rows);
                    emit(AtlasEvent::Country {
                        generation,
                        code,
                        rows,
                        from_cache: false,
                    });
                }
                Err(error) => emit(AtlasEvent::Error {
                    generation,
                    kind: AtlasErrorKind::Network,
                    message: format!("{error:#}"),
                }),
            }
        }
        AtlasCmd::Search { generation, query } => {
            let q = RadioQuery::Search {
                name: query.clone(),
                limit: 80,
            };
            match request_bounded(fast, &q).await {
                Ok(json) => emit(AtlasEvent::Search {
                    generation,
                    query,
                    rows: radio_browser::parse_station_list(&json, 80),
                }),
                Err(error) => emit(AtlasEvent::Error {
                    generation,
                    kind: AtlasErrorKind::Network,
                    message: format!("{error:#}"),
                }),
            }
        }
        AtlasCmd::Resolve { generation, uuids } => {
            let mut rows = Vec::new();
            let uuids: Vec<String> = uuids
                .into_iter()
                .filter(|uuid| radio_browser::is_directory_uuid(uuid))
                .collect();
            for chunk in uuids.chunks(100) {
                let Some(query) = RadioQuery::by_uuid(chunk.to_vec()) else {
                    continue;
                };
                match request_bounded(fast, &query).await {
                    Ok(json) => {
                        rows.extend(radio_browser::parse_station_list(&json, MAX_CACHED_ROWS))
                    }
                    Err(error) => {
                        emit(AtlasEvent::Error {
                            generation,
                            kind: AtlasErrorKind::Network,
                            message: format!("{error:#}"),
                        });
                        return;
                    }
                }
            }
            emit(AtlasEvent::Resolved { generation, rows });
        }
        AtlasCmd::Click { .. } => {}
    }
}

/// Mirror fallback is bounded as a whole: a directory outage costs at most one minute of
/// the actor's time, after which the interactive kinds get their turn.
async fn request_bounded(
    client: &reqwest::Client,
    query: &RadioQuery,
) -> anyhow::Result<serde_json::Value> {
    match tokio::time::timeout(REQUEST_DEADLINE, radio_browser::request(client, query)).await {
        Ok(result) => result,
        Err(_) => anyhow::bail!("Radio Browser request timed out"),
    }
}

fn store_rows(kind: &cache::CacheKind<'_>, rows: &[RadioStation]) {
    if let Err(error) = cache::store(kind, rows) {
        tracing::debug!(error = %format!("{error:#}"), "atlas cache store failed");
    }
}

async fn request_click(client: &reqwest::Client, uuid: &str) -> anyhow::Result<()> {
    let Some(query) = RadioQuery::click(uuid) else {
        anyhow::bail!("invalid station uuid");
    };
    request_bounded(client, &query).await?;
    Ok(())
}

pub mod cache {
    use super::*;

    #[derive(Clone, Copy, Debug)]
    pub enum CacheKind<'a> {
        World,
        Country(&'a str),
    }

    #[derive(Serialize, Deserialize)]
    struct CacheFile {
        saved_unix: u64,
        rows: Vec<RadioStation>,
    }

    #[derive(Clone, Debug, PartialEq)]
    pub struct Loaded {
        pub rows: Vec<RadioStation>,
        pub fresh: bool,
    }

    pub fn dir() -> anyhow::Result<PathBuf> {
        let dir = crate::paths::data_dir()
            .ok_or_else(|| anyhow::anyhow!("no data directory"))?
            .join("atlas");
        safe_fs::ensure_private_dir(&dir)?;
        Ok(dir)
    }

    pub fn path(kind: &CacheKind<'_>) -> anyhow::Result<PathBuf> {
        Ok(match kind {
            CacheKind::World => dir()?.join("world.json"),
            CacheKind::Country(code) => {
                let cc: String = code
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric())
                    .take(3)
                    .collect();
                dir()?.join("countries").join(format!("{cc}.json"))
            }
        })
    }

    pub fn store(kind: &CacheKind<'_>, rows: &[RadioStation]) -> anyhow::Result<()> {
        let path = path(kind)?;
        if let Some(parent) = path.parent() {
            safe_fs::ensure_private_dir(parent)?;
        }
        let saved_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        safe_fs::write_private_atomic_json(
            &path,
            &CacheFile {
                saved_unix,
                rows: rows.to_vec(),
            },
        )?;
        Ok(())
    }

    pub fn load(kind: &CacheKind<'_>) -> anyhow::Result<Option<Loaded>> {
        let path = path(kind)?;
        // Anything unreadable — missing, oversized, a symlink, wrong owner — is simply not a
        // cache hit; the network path takes over and the next store overwrites it.
        let bytes = match safe_fs::read_no_symlink_limited(&path, ATLAS_JSON_MAX as u64) {
            Ok(bytes) => bytes,
            Err(error) => {
                if error.kind() != std::io::ErrorKind::NotFound {
                    tracing::debug!(%error, path = %path.display(), "atlas cache unreadable");
                }
                return Ok(None);
            }
        };
        let file: CacheFile = match serde_json::from_slice(&bytes) {
            Ok(file) => file,
            Err(_) => return Ok(None),
        };
        if file.rows.len() > MAX_CACHED_ROWS {
            return Ok(None);
        }
        // A cache file is user-writable data: re-validate every row exactly as a network row
        // would be, dropping anything the parser would have rejected.
        let rows: Vec<RadioStation> = file
            .rows
            .into_iter()
            .filter(RadioStation::is_valid)
            .collect();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let fresh = now.saturating_sub(file.saved_unix) < CACHE_TTL.as_secs();
        Ok(Some(Loaded { rows, fresh }))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn station(uuid: &str) -> RadioStation {
            RadioStation::parse(&serde_json::json!({
                "stationuuid": uuid,
                "url": "https://stream.example.org/live",
                "name": uuid,
                "countrycode": "KR",
            }))
            .unwrap()
        }

        #[test]
        fn round_trip_and_ttl() {
            let rows = vec![station("a"), station("b")];
            let kind = CacheKind::Country("T1");
            let path = path(&kind).unwrap();
            std::fs::remove_file(&path).ok();
            store(&kind, &rows).unwrap();
            let loaded = load(&kind).unwrap().unwrap();
            assert_eq!(loaded.rows, rows);
            assert!(loaded.fresh);
            std::fs::remove_file(&path).ok();
        }

        #[test]
        fn stale_entries_are_not_fresh() {
            let kind = CacheKind::Country("T2");
            let path = path(&kind).unwrap();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, br#"{"saved_unix": 0, "rows": []}"#).unwrap();
            let loaded = load(&kind).unwrap().unwrap();
            assert!(!loaded.fresh);
            std::fs::remove_file(&path).ok();
        }

        #[test]
        fn malformed_and_oversized_are_absent() {
            let kind = CacheKind::Country("T3");
            let path = path(&kind).unwrap();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"not json").unwrap();
            assert!(load(&kind).unwrap().is_none());
            std::fs::write(&path, vec![b'x'; ATLAS_JSON_MAX + 1]).unwrap();
            assert!(load(&kind).unwrap().is_none());
            std::fs::remove_file(&path).ok();
        }
    }
}
