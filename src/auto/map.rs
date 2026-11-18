//! The offline world map of the location picker (pure): decoding of the embedded
//! coastline data and the zoom/pan view of an equirectangular projection.

/// Highest zoom of the picker (1 shows the whole world).
pub const MAX_ZOOM: f64 = 8.0;

/// Land polygons as rings of `(lon, lat)` degrees. Format of `assets/world-110m.bin`
/// (little-endian): `u16` ring count, then per ring a `u16` point count and that many
/// `(lon, lat)` pairs of `i16` hundredths of a degree. The first point is absolute, the
/// others are deltas to the previous one (wrapping). `None` if the data is truncated.
pub fn decode_rings(data: &[u8]) -> Option<Vec<Vec<(f32, f32)>>> {
    let mut at = 0;
    let u16_at = |at: &mut usize| {
        let v = u16::from_le_bytes([*data.get(*at)?, *data.get(*at + 1)?]);
        *at += 2;
        Some(v)
    };
    let rings = u16_at(&mut at)?;
    let mut out = Vec::with_capacity(rings.into());
    for _ in 0..rings {
        let n = u16_at(&mut at)?;
        let (mut x, mut y) = (0i16, 0i16);
        let mut ring = Vec::with_capacity(n.into());
        for _ in 0..n {
            x = x.wrapping_add(u16_at(&mut at)? as i16);
            y = y.wrapping_add(u16_at(&mut at)? as i16);
            ring.push((f32::from(x) / 100.0, f32::from(y) / 100.0));
        }
        out.push(ring);
    }
    Some(out)
}

/// Round to the two decimals (about 1 km) the location is stored with.
pub fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// What part of the world is shown in a `w`×`h` pixel viewport: the center and the zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub lon: f64,
    pub lat: f64,
    pub zoom: f64,
}

impl Default for View {
    fn default() -> View {
        View {
            lon: 0.0,
            lat: 0.0,
            zoom: 1.0,
        }
    }
}

impl View {
    /// Pixels per degree: zoom 1 fits the whole world in the viewport.
    pub fn scale(&self, w: f64, h: f64) -> f64 {
        (w / 360.0).min(h / 180.0) * self.zoom
    }

    pub fn to_screen(&self, lon: f64, lat: f64, w: f64, h: f64) -> (f64, f64) {
        let s = self.scale(w, h);
        (
            w / 2.0 + (lon - self.lon) * s,
            h / 2.0 - (lat - self.lat) * s,
        )
    }

    /// The coordinates under a viewport pixel, limited to the valid range and rounded.
    pub fn to_geo(&self, x: f64, y: f64, w: f64, h: f64) -> (f64, f64) {
        let s = self.scale(w, h);
        let lon = self.lon + (x - w / 2.0) / s;
        let lat = self.lat - (y - h / 2.0) / s;
        (
            round2(lon.clamp(-180.0, 180.0)),
            round2(lat.clamp(-90.0, 90.0)),
        )
    }

    /// Keep the view inside the world (centered on an axis that is fully visible).
    fn clamped(mut self, w: f64, h: f64) -> View {
        self.zoom = self.zoom.clamp(1.0, MAX_ZOOM);
        let s = self.scale(w, h);
        let limit = |half_view: f64, max: f64| (max - half_view).max(0.0);
        let (dx, dy) = (limit(w / 2.0 / s, 180.0), limit(h / 2.0 / s, 90.0));
        self.lon = self.lon.clamp(-dx, dx);
        self.lat = self.lat.clamp(-dy, dy);
        self
    }

    /// Multiply the zoom by `factor`, keeping the point under pixel `(x, y)` in place.
    pub fn zoom_at(self, x: f64, y: f64, w: f64, h: f64, factor: f64) -> View {
        let (lon, lat) = (
            self.lon + (x - w / 2.0) / self.scale(w, h),
            self.lat - (y - h / 2.0) / self.scale(w, h),
        );
        let mut v = self;
        v.zoom = (self.zoom * factor).clamp(1.0, MAX_ZOOM);
        let s = v.scale(w, h);
        v.lon = lon - (x - w / 2.0) / s;
        v.lat = lat + (y - h / 2.0) / s;
        v.clamped(w, h)
    }

    /// Drag the map by `(dx, dy)` pixels.
    pub fn pan(self, dx: f64, dy: f64, w: f64, h: f64) -> View {
        let s = self.scale(w, h);
        View {
            lon: self.lon - dx / s,
            lat: self.lat + dy / s,
            ..self
        }
        .clamped(w, h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORLD: &[u8] = include_bytes!("../../assets/world-110m.bin");

    #[test]
    fn world_data_decodes_and_stays_on_the_globe() {
        let rings = decode_rings(WORLD).unwrap();
        assert!(rings.len() > 50);
        assert!(rings.iter().all(|r| r.len() >= 3));
        for &(lon, lat) in rings.iter().flatten() {
            assert!((-180.0..=180.0).contains(&lon) && (-90.0..=90.0).contains(&lat));
        }
        // Antarctica reaches the south pole.
        let min_lat = rings.iter().flatten().map(|p| p.1).fold(90.0, f32::min);
        assert!(min_lat < -80.0);
    }

    #[test]
    fn truncated_data_is_rejected() {
        assert!(decode_rings(&WORLD[..WORLD.len() - 1]).is_none());
        assert!(decode_rings(&[]).is_none());
    }

    #[test]
    fn decodes_deltas_with_wraparound() {
        // One ring of 3 points: (-180, 0), (180, 0) as a wrapped delta (+36000), (180, 50).
        let mut d = vec![1, 0, 3, 0];
        for v in [-18000i16, 0, -29536, 0, 0, 5000] {
            d.extend(v.to_le_bytes());
        }
        let r = decode_rings(&d).unwrap();
        assert_eq!(r[0], [(-180.0, 0.0), (180.0, 0.0), (180.0, 50.0)]);
    }

    #[test]
    fn projection_round_trips() {
        let v = View::default();
        let (w, h) = (720.0, 360.0);
        let (x, y) = v.to_screen(12.5, 41.9, w, h);
        assert_eq!(v.to_geo(x, y, w, h), (12.5, 41.9));
        assert_eq!(v.to_geo(w / 2.0, h / 2.0, w, h), (0.0, 0.0));
        // Outside the map clamps to the valid range.
        assert_eq!(v.to_geo(-50.0, 999.0, w, h), (-180.0, -90.0));
    }

    #[test]
    fn zoom_keeps_the_cursor_point_and_limits() {
        let (w, h) = (720.0, 360.0);
        let v = View::default().zoom_at(500.0, 100.0, w, h, 2.0);
        assert_eq!(v.zoom, 2.0);
        let (lon, lat) = View::default().to_geo(500.0, 100.0, w, h);
        let (x, y) = v.to_screen(lon, lat, w, h);
        assert!((x - 500.0).abs() < 0.5 && (y - 100.0).abs() < 0.5);
        let max = (0..10).fold(View::default(), |v, _| v.zoom_at(360.0, 180.0, w, h, 2.0));
        assert_eq!(max.zoom, MAX_ZOOM);
        let min = max.zoom_at(360.0, 180.0, w, h, 0.01);
        assert_eq!(min, View::default());
    }

    #[test]
    fn pan_stays_inside_the_world() {
        let (w, h) = (720.0, 360.0);
        // At zoom 1 the whole world is visible: nothing to pan.
        assert_eq!(View::default().pan(100.0, 50.0, w, h), View::default());
        let z = View::default().zoom_at(360.0, 180.0, w, h, 4.0);
        let p = z.pan(-10_000.0, 10_000.0, w, h);
        // Dragging far left and down ends at the east and north edges.
        assert!((p.lon - 135.0).abs() < 1e-9 && (p.lat - 67.5).abs() < 1e-9);
    }
}
