//! Camera orientation, inverse projection, and deterministic drag kinetics for Atlas.

use crate::atlas::{LatLon, wrap_longitude};

pub const MIN_LAT: f32 = -78.0;
pub const MAX_LAT: f32 = 78.0;
pub const MIN_SCALE: f32 = 0.72;
pub const MAX_SCALE: f32 = 12.0;
pub const DEFAULT_CENTRE: LatLon = LatLon::new(18.0, -20.0);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub centre: LatLon,
    pub scale: f32,
}

/// Trigonometric camera state shared by the projection and the raster hot loop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraFrame {
    pub sin_lat: f32,
    pub cos_lat: f32,
    pub sin_lon: f32,
    pub cos_lon: f32,
    pub centre_lat_deg: f32,
    pub centre_lon_deg: f32,
}

impl Camera {
    pub const fn new() -> Self {
        Self {
            centre: DEFAULT_CENTRE,
            scale: 1.0,
        }
    }

    pub fn rotate_by(&mut self, dlat: f32, dlon: f32) {
        if dlat.is_finite() {
            self.centre.lat = (self.centre.lat + dlat).clamp(MIN_LAT, MAX_LAT);
        }
        if dlon.is_finite() {
            self.centre.lon = canonical_longitude(self.centre.lon + dlon);
        }
    }

    pub fn zoom_by(&mut self, factor: f32) {
        if factor.is_finite() {
            self.scale = (self.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
        }
    }

    pub fn focus(&mut self, at: LatLon) {
        if at.lat.is_finite() {
            self.centre.lat = at.lat.clamp(MIN_LAT, MAX_LAT);
        }
        if at.lon.is_finite() {
            self.centre.lon = canonical_longitude(at.lon);
        }
    }

    /// Projects through the shared `LatLon::to_unit` coordinate convention without pointwise
    /// trigonometry.
    #[inline]
    pub fn project_unit(&self, unit: [f32; 3]) -> (f32, f32, f32) {
        let frame = self.frame();
        project_with_frame(unit, frame)
    }

    #[inline]
    pub fn frame(&self) -> CameraFrame {
        let (sin_lat, cos_lat) = self.centre.lat.to_radians().sin_cos();
        let (sin_lon, cos_lon) = self.centre.lon.to_radians().sin_cos();
        CameraFrame {
            sin_lat,
            cos_lat,
            sin_lon,
            cos_lon,
            centre_lat_deg: self.centre.lat,
            centre_lon_deg: self.centre.lon,
        }
    }

    pub fn unproject(&self, nx: f32, ny: f32) -> Option<LatLon> {
        let rho_squared = nx.mul_add(nx, ny * ny);
        if !rho_squared.is_finite() || rho_squared > 1.0 {
            return None;
        }

        let frame = self.frame();
        let nz = (1.0 - rho_squared).max(0.0).sqrt();
        let lat = (ny.mul_add(frame.cos_lat, nz * frame.sin_lat))
            .clamp(-1.0, 1.0)
            .asin()
            .to_degrees();
        let lon = frame.centre_lon_deg
            + nx.atan2(nz.mul_add(frame.cos_lat, -ny * frame.sin_lat))
                .to_degrees();
        Some(LatLon::new(lat, canonical_longitude(lon)))
    }
}

#[inline]
fn canonical_longitude(lon: f32) -> f32 {
    let wrapped = wrap_longitude(lon);
    if wrapped == -180.0 { 180.0 } else { wrapped }
}

impl Default for Camera {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
pub(crate) fn project_with_frame(unit: [f32; 3], frame: CameraFrame) -> (f32, f32, f32) {
    let horizontal = unit[0].mul_add(frame.cos_lon, unit[1] * frame.sin_lon);
    let x = unit[1].mul_add(frame.cos_lon, -unit[0] * frame.sin_lon);
    let y = frame.cos_lat.mul_add(unit[2], -frame.sin_lat * horizontal);
    let depth = frame.sin_lat.mul_add(unit[2], frame.cos_lat * horizontal);
    (x, y, depth)
}

pub const KINETIC_LAUNCH_SPEED: f32 = 120.0;
pub const KINETIC_MAX_SPEED: f32 = 2400.0;
pub const KINETIC_DECELERATION: f32 = 1800.0;
pub const KINETIC_MAX_FRAME: f32 = 0.1;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Kinetic {
    pub vx: f32,
    pub vy: f32,
}

impl Kinetic {
    pub fn launch(vx: f32, vy: f32) -> Option<Self> {
        let speed = vx.hypot(vy);
        if !speed.is_finite() || speed < KINETIC_LAUNCH_SPEED {
            return None;
        }

        let limit = (KINETIC_MAX_SPEED / speed).min(1.0);
        Some(Self {
            vx: vx * limit,
            vy: vy * limit,
        })
    }

    #[inline]
    pub fn active(&self) -> bool {
        let speed = self.vx.hypot(self.vy);
        speed.is_finite() && speed > 1e-6
    }

    pub fn step(&mut self, cam: &mut Camera, dt: f32, radius_dots: f32) -> bool {
        if !dt.is_finite() || dt <= 0.0 || dt > KINETIC_MAX_FRAME {
            *self = Self::default();
            return false;
        }

        let speed = self.vx.hypot(self.vy);
        if !speed.is_finite() || speed <= 1e-6 {
            *self = Self::default();
            return false;
        }

        let active_time = dt.min(speed / KINETIC_DECELERATION);
        let next_speed = (speed - KINETIC_DECELERATION * active_time).max(0.0);
        let distance_scale = (speed + next_speed) * active_time / (2.0 * speed);
        let dx = self.vx * distance_scale;
        let dy = self.vy * distance_scale;
        let latitude_before = cam.centre.lat;

        cam.rotate_by(
            dy * degrees_per_dot(radius_dots),
            -dx * degrees_per_dot(radius_dots),
        );

        let velocity_scale = next_speed / speed;
        self.vx *= velocity_scale;
        self.vy *= velocity_scale;
        let pushed_past_latitude_bound = (dy > 0.0
            && (latitude_before >= MAX_LAT || cam.centre.lat >= MAX_LAT))
            || (dy < 0.0 && (latitude_before <= MIN_LAT || cam.centre.lat <= MIN_LAT));
        if pushed_past_latitude_bound {
            self.vy = 0.0;
        }
        if !self.active() {
            *self = Self::default();
            return false;
        }
        true
    }
}

/// At the centre of the disc, a pointer arc length maps directly to this angular distance.
#[inline]
pub fn degrees_per_dot(radius_dots: f32) -> f32 {
    let radius = if radius_dots.is_finite() {
        radius_dots.max(1.0)
    } else {
        1.0
    };
    180.0 / (std::f32::consts::PI * radius)
}

const VELOCITY_WINDOW: f32 = 0.1;
const MAX_MOTION_SAMPLE: f32 = 0.25;

#[derive(Clone, Copy, Debug)]
struct MotionSample {
    at: f32,
    vx: f32,
    vy: f32,
}

/// Deterministic pointer-motion smoothing. Timestamps are supplied by the caller so tests and
/// UI event replay do not depend on a wall clock.
#[derive(Clone, Debug, Default)]
pub struct VelocityTracker {
    samples: Vec<MotionSample>,
}

impl VelocityTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a displacement ending at `at`, in seconds from a caller-owned monotonic epoch.
    pub fn record(&mut self, dx: f32, dy: f32, dt: f32, at: f32) {
        self.samples
            .retain(|sample| at - sample.at <= VELOCITY_WINDOW);
        if !dx.is_finite()
            || !dy.is_finite()
            || !dt.is_finite()
            || !at.is_finite()
            || dt <= 0.0
            || dt > MAX_MOTION_SAMPLE
        {
            return;
        }

        let instant_vx = dx / dt;
        let instant_vy = dy / dt;
        let (vx, vy) = self
            .samples
            .last()
            .map_or((instant_vx, instant_vy), |previous| {
                (
                    previous.vx * 0.25 + instant_vx * 0.75,
                    previous.vy * 0.25 + instant_vy * 0.75,
                )
            });
        self.samples.push(MotionSample { at, vx, vy });
    }

    pub fn release(&self, now: f32) -> (f32, f32) {
        if !now.is_finite() {
            return (0.0, 0.0);
        }
        self.samples.last().map_or((0.0, 0.0), |sample| {
            if now - sample.at <= VELOCITY_WINDOW {
                (sample.vx, sample.vy)
            } else {
                (0.0, 0.0)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(left: f32, right: f32, tolerance: f32) {
        assert!(
            (left - right).abs() <= tolerance,
            "{left} was not within {tolerance} of {right}"
        );
    }

    #[test]
    fn camera_clamps_and_wraps() {
        let mut camera = Camera::new();
        camera.rotate_by(1_000.0, 200.0);
        assert_eq!(camera.centre.lat, MAX_LAT);
        assert_eq!(camera.centre.lon, 180.0);
        camera.focus(LatLon::new(-1_000.0, -180.0));
        assert_eq!(camera.centre.lat, MIN_LAT);
        assert_eq!(camera.centre.lon, 180.0);
        camera.zoom_by(100.0);
        assert_eq!(camera.scale, MAX_SCALE);
        camera.zoom_by(0.0);
        assert_eq!(camera.scale, MIN_SCALE);
    }

    #[test]
    fn projection_matches_camera_axes() {
        let mut camera = Camera::new();
        camera.focus(LatLon::new(0.0, 0.0));
        let centre = camera.project_unit(LatLon::new(0.0, 0.0).to_unit());
        let east = camera.project_unit(LatLon::new(0.0, 90.0).to_unit());
        let north = camera.project_unit(LatLon::new(90.0, 0.0).to_unit());
        close(centre.0, 0.0, 1e-6);
        close(centre.1, 0.0, 1e-6);
        close(centre.2, 1.0, 1e-6);
        close(east.0, 1.0, 1e-6);
        close(north.1, 1.0, 1e-6);
    }

    #[test]
    fn projection_unprojection_round_trips_visible_lattice() {
        for centre in [
            LatLon::new(0.0, 0.0),
            LatLon::new(35.0, 67.0),
            LatLon::new(-55.0, -123.0),
        ] {
            let mut camera = Camera::new();
            camera.focus(centre);
            for lat in (-80..=80).step_by(10) {
                for lon in (-170..=180).step_by(10) {
                    let source = LatLon::new(lat as f32, lon as f32);
                    let (x, y, depth) = camera.project_unit(source.to_unit());
                    if depth > 0.05 {
                        let recovered = camera
                            .unproject(x, y)
                            .expect("a projected visible point is inside the disc");
                        close(recovered.lat, source.lat, 1e-3);
                        close(wrap_longitude(recovered.lon - source.lon), 0.0, 1e-3);
                    }
                }
            }
        }
        assert!(Camera::new().unproject(1.1, 0.0).is_none());
    }

    #[test]
    fn kinetic_launch_clamps_and_coast_lands() {
        assert!(Kinetic::launch(119.0, 0.0).is_none());
        let launch = Kinetic::launch(9_000.0, 0.0).expect("fast enough to launch");
        close(launch.vx.hypot(launch.vy), KINETIC_MAX_SPEED, 1e-3);

        let mut camera = Camera::new();
        let initial_lon = camera.centre.lon;
        let mut coast = Kinetic::launch(600.0, 0.0).expect("fast enough to coast");
        let mut steps = 0;
        while coast.step(&mut camera, 1.0 / 60.0, 80.0) {
            steps += 1;
            assert!(steps < 100, "coast did not settle");
        }
        assert!(steps > 0);
        assert!(initial_lon - camera.centre.lon > 0.0);
    }

    #[test]
    fn kinetic_stops_for_bad_frame_and_latitude_clamp() {
        let mut camera = Camera {
            centre: LatLon::new(MAX_LAT - 0.1, 0.0),
            scale: 1.0,
        };
        let mut kinetic = Kinetic { vx: 0.0, vy: 500.0 };
        assert!(!kinetic.step(&mut camera, 0.1, 100.0));
        assert_eq!(camera.centre.lat, MAX_LAT);
        assert_eq!(kinetic.vy, 0.0);

        let mut kinetic = Kinetic { vx: 600.0, vy: 0.0 };
        let before = camera.centre;
        assert!(!kinetic.step(&mut camera, 0.2, 100.0));
        assert_eq!(camera.centre, before);
    }

    #[test]
    fn velocity_tracker_uses_recent_ema_samples() {
        let mut tracker = VelocityTracker::new();
        tracker.record(10.0, 0.0, 0.01, 0.01);
        tracker.record(20.0, 10.0, 0.02, 0.03);
        let (vx, vy) = tracker.release(0.08);
        close(vx, 1_000.0, 1e-4);
        close(vy, 375.0, 1e-4);
        assert_eq!(tracker.release(0.131), (0.0, 0.0));
        tracker.record(1_000.0, 0.0, 0.3, 0.4);
        assert_eq!(tracker.release(0.4), (0.0, 0.0));
    }
}
