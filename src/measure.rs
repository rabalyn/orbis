//! Persistent measurement trails and their altitude-aware distance summaries.

use crate::import::{Imported, Item, Pos, Shape};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub name: String,
    pub points: Vec<Pos>,
    pub visible: bool,
    pub colour: [u8; 4],
    pub width: f32,
    pub dotted: bool,
}

impl Track {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            points: Vec::new(),
            visible: true,
            colour: [255, 80, 40, 240],
            width: 3.0,
            dotted: false,
        }
    }

    /// WGS84-like spherical surface distance combined with measured altitude change.
    pub fn legs(&self) -> Vec<(f64, f64)> {
        self.points
            .windows(2)
            .map(|p| {
                let dlat = p[1].lat - p[0].lat;
                let dlon = (p[1].lon - p[0].lon + std::f64::consts::PI)
                    .rem_euclid(2.0 * std::f64::consts::PI)
                    - std::f64::consts::PI;
                let a = (dlat / 2.0).sin().powi(2)
                    + p[0].lat.cos() * p[1].lat.cos() * (dlon / 2.0).sin().powi(2);
                let h0 = p[0].alt.unwrap_or(0.0);
                let h1 = p[1].alt.unwrap_or(0.0);
                let horizontal = 2.0 * (crate::geo::radius() + (h0 + h1) * 0.5) * a.sqrt().asin();
                let climb = h1 - h0;
                (horizontal.hypot(climb), climb)
            })
            .collect()
    }

    pub fn totals(&self) -> (f64, f64, f64) {
        self.legs()
            .into_iter()
            .fold((0.0, 0.0, 0.0), |(all, up, down), (d, dz)| {
                (
                    all + d,
                    up + if dz > 0.0 { d } else { 0.0 },
                    down + if dz < 0.0 { d } else { 0.0 },
                )
            })
    }

    pub fn imported(&self) -> Imported {
        let mut item = Item::new(&self.name, Shape::Line(self.points.clone()));
        item.outline = Some(crate::import::Outline {
            edge: self.colour,
            fill: self.colour,
            width: self.width,
            dashed: self.dotted,
        });
        Imported {
            name: self.name.clone(),
            items: vec![item],
            truncated: false,
        }
    }

    /// GPX 1.1 representation, retaining the measured elevations.
    pub fn to_gpx(&self) -> String {
        let name = self
            .name
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;");
        let mut xml = format!("<?xml version=\"1.0\"?><gpx version=\"1.1\" creator=\"Orbis\" xmlns=\"http://www.topografix.com/GPX/1/1\"><trk><name>{name}</name><trkseg>");
        for p in &self.points {
            xml.push_str(&format!(
                "<trkpt lat=\"{:.8}\" lon=\"{:.8}\">",
                p.lat.to_degrees(),
                p.lon.to_degrees()
            ));
            if let Some(alt) = p.alt {
                xml.push_str(&format!("<ele>{alt:.2}</ele>"));
            }
            xml.push_str("</trkpt>");
        }
        xml.push_str("</trkseg></trk></gpx>");
        xml
    }

    fn json(&self) -> Value {
        json!({"name":self.name,"visible":self.visible,"colour":self.colour,"width":self.width,"dotted":self.dotted,
            "points":self.points.iter().map(|p| json!([p.lat.to_degrees(),p.lon.to_degrees(),p.alt])).collect::<Vec<_>>()})
    }

    fn parse(v: &Value) -> Option<Self> {
        let points = v
            .get("points")?
            .as_array()?
            .iter()
            .filter_map(|p| {
                let a = p.as_array()?;
                let lat = a.first()?.as_f64()?;
                let lon = a.get(1)?.as_f64()?;
                if !lat.is_finite() || !lon.is_finite() || lat.abs() > 90.0 || lon.abs() > 180.0 {
                    return None;
                }
                let alt = a
                    .get(2)
                    .filter(|x| !x.is_null())
                    .and_then(Value::as_f64)
                    .filter(|x| x.is_finite());
                Some(Pos {
                    lat: lat.to_radians(),
                    lon: lon.to_radians(),
                    alt,
                })
            })
            .collect::<Vec<_>>();
        let mut t = Self::new(v.get("name")?.as_str()?);
        t.points = points;
        t.visible = v.get("visible").and_then(Value::as_bool).unwrap_or(true);
        if let Some(c) = v
            .get("colour")
            .and_then(Value::as_array)
            .filter(|a| a.len() == 4)
        {
            for (i, n) in c.iter().enumerate() {
                t.colour[i] = n.as_u64().unwrap_or(255).min(255) as u8;
            }
        }
        t.width = v
            .get("width")
            .and_then(Value::as_f64)
            .filter(|x| x.is_finite())
            .unwrap_or(3.0)
            .clamp(1.0, 12.0) as f32;
        t.dotted = v.get("dotted").and_then(Value::as_bool).unwrap_or(false);
        Some(t)
    }
}

pub struct Library {
    path: Option<std::path::PathBuf>,
    pub tracks: Vec<Track>,
    pub generation: u64,
}

impl Library {
    pub fn load(path: Option<std::path::PathBuf>) -> Self {
        let tracks = path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| v.get("tracks")?.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(Track::parse)
            .collect();
        Self {
            path,
            tracks,
            generation: 0,
        }
    }
    pub fn save(&self) {
        let Some(path) = &self.path else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let value =
            json!({"version":1,"tracks":self.tracks.iter().map(Track::json).collect::<Vec<_>>()});
        let _ = std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap_or_default());
    }

    pub fn clone_track(&mut self, index: usize) -> Option<usize> {
        let mut copy = self.tracks.get(index)?.clone();
        copy.name.push_str(" copy");
        self.tracks.push(copy);
        Some(self.tracks.len() - 1)
    }

    pub fn delete_track(&mut self, index: usize) -> bool {
        if index >= self.tracks.len() {
            return false;
        }
        self.tracks.remove(index);
        true
    }

    pub fn visible_docs(&self) -> impl Iterator<Item = Imported> + '_ {
        self.tracks
            .iter()
            .filter(|t| t.visible)
            .map(Track::imported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(lat: f64, lon: f64, alt: Option<f64>) -> Pos {
        Pos {
            lat: lat.to_radians(),
            lon: lon.to_radians(),
            alt,
        }
    }

    fn close(a: f64, b: f64, tolerance: f64) {
        assert!((a - b).abs() <= tolerance, "{a} != {b} within {tolerance}");
    }

    #[test]
    fn surface_distance_and_vertical_change_form_a_3d_leg() {
        let mut t = Track::new("ridge");
        t.points = vec![pos(0.0, 0.0, Some(0.0)), pos(0.0, 0.0, Some(300.0))];
        assert_eq!(t.legs(), vec![(300.0, 300.0)]);

        t.points = vec![pos(0.0, 0.0, Some(0.0)), pos(0.0, 1.0, Some(0.0))];
        close(t.legs()[0].0, crate::geo::radius().to_radians(), 1e-8);
    }

    #[test]
    fn totals_separate_uphill_downhill_and_flat_legs() {
        let mut t = Track::new("climb");
        t.points = vec![
            pos(0.0, 0.0, Some(0.0)),
            pos(0.0, 0.0, Some(100.0)),
            pos(0.0, 0.0, Some(100.0)),
            pos(0.0, 0.0, Some(0.0)),
        ];
        assert_eq!(t.totals(), (200.0, 100.0, 100.0));
    }

    #[test]
    fn antimeridian_leg_uses_the_short_way_round() {
        let mut t = Track::new("date line");
        t.points = vec![pos(0.0, 179.9, None), pos(0.0, -179.9, None)];
        close(
            t.legs()[0].0,
            crate::geo::radius() * 0.2_f64.to_radians(),
            1e-7,
        );
    }

    #[test]
    fn style_and_visibility_are_transferred_to_render_docs() {
        let mut t = Track::new("styled");
        t.points = vec![pos(1.0, 2.0, None), pos(2.0, 3.0, Some(20.0))];
        t.colour = [1, 2, 3, 4];
        t.width = 7.0;
        t.dotted = true;
        let item = &t.imported().items[0];
        assert_eq!(
            item.outline,
            Some(crate::import::Outline {
                edge: [1, 2, 3, 4],
                fill: [1, 2, 3, 4],
                width: 7.0,
                dashed: true
            })
        );

        let mut lib = Library {
            path: None,
            tracks: vec![t],
            generation: 0,
        };
        assert_eq!(lib.visible_docs().count(), 1);
        lib.tracks[0].visible = false;
        assert_eq!(lib.visible_docs().count(), 0);
    }

    #[test]
    fn clone_and_delete_manage_library_entries() {
        let mut lib = Library {
            path: None,
            tracks: vec![Track::new("walk")],
            generation: 0,
        };
        assert_eq!(lib.clone_track(0), Some(1));
        assert_eq!(lib.tracks[1].name, "walk copy");
        assert!(lib.delete_track(0));
        assert_eq!(lib.tracks.len(), 1);
        assert!(!lib.delete_track(4));
        assert_eq!(lib.clone_track(4), None);
    }

    #[test]
    fn persistence_round_trips_track_metadata_and_points() {
        let path = std::env::temp_dir().join(format!("orbis-measure-{}.json", std::process::id()));
        let mut t = Track::new("A & <B>");
        t.points = vec![pos(48.1, 11.5, Some(523.4)), pos(48.2, 11.7, None)];
        t.visible = false;
        t.colour = [4, 5, 6, 7];
        t.width = 5.5;
        t.dotted = true;
        let lib = Library {
            path: Some(path.clone()),
            tracks: vec![t],
            generation: 0,
        };
        lib.save();
        let loaded = Library::load(Some(path.clone()));
        assert_eq!(loaded.tracks, lib.tracks);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn gpx_export_import_round_trips_name_coordinates_and_altitude() {
        let mut t = Track::new("A & <ridge>");
        t.points = vec![pos(46.0, 11.0, Some(558.25)), pos(46.1, 11.2, None)];
        let parsed = crate::import::gpx(&t.to_gpx(), "export.gpx").unwrap();
        assert_eq!(parsed.items.len(), 1);
        assert_eq!(parsed.items[0].name, "A & <ridge>");
        let Shape::Line(points) = &parsed.items[0].shape else {
            panic!("GPX line expected")
        };
        close(points[0].lat, t.points[0].lat, 1e-8);
        close(points[0].lon, t.points[0].lon, 1e-8);
        assert_eq!(points[0].alt, Some(558.25));
        assert_eq!(points[1].alt, None);
    }

    #[test]
    fn color_editor_round_trips_do_not_premultiply_alpha_repeatedly() {
        // Mirrors the editor's per-frame conversion between stored sRGBA and egui Color32.
        // Color32's r/g/b accessors are premultiplied; using them here made each frame darker.
        let mut stored = [240, 120, 60, 128];
        let mut stable = None;
        for _ in 0..120 {
            let color = egui::Color32::from_rgba_unmultiplied(
                stored[0], stored[1], stored[2], stored[3],
            );
            stored = color.to_srgba_unmultiplied();
            if let Some(expected) = stable {
                assert_eq!(stored, expected, "color drifted between UI frames");
            } else {
                stable = Some(stored);
            }
        }
        assert_ne!(stored, [0, 0, 0, 128]);
    }
}
