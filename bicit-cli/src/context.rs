use std::cell::RefCell;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use anyhow::anyhow;
use anyhow::Result;
use chrono::Duration;
use geo::algorithm::line_measures::Length;
use geo::{Distance, Geodesic, MultiLineString};
use geo_types::Point;
use gpx::read;
use hhmmss::Hhmmss;

use crate::map::render_track_map_href;
use crate::InputPath;

#[derive(Debug)]
struct ElevPoint {
    e: f64,
    d: f64,
}

#[derive(Debug)]
struct ContextData {
    track_name: String,
    distance: f64,
    speed: f64,
    speed_max: f64,
    speed_moving: f64,
    time: Duration,
    time_moving: Duration,
    uphill: f64,
    downhill: f64,
    elevation: Vec<ElevPoint>,
    elevation_max: f64,
    elevation_min: f64,
    coords: Vec<Point<f64>>,
}

pub struct Context<'a> {
    filename: &'a str,

    data: Option<ContextData>,
    map_href: RefCell<Option<String>>,
    map_size: RefCell<Option<(u32, u32)>>,
    map_track_color: RefCell<Option<galileo::Color>>,
}

impl Context<'_> {
    pub fn new(filename: &str) -> Context<'_> {
        Context {
            filename,
            data: None,
            map_href: RefCell::new(None),
            map_size: RefCell::new(None),
            map_track_color: RefCell::new(None),
        }
    }

    pub fn cleanup_temp_files(&self) {
        self.map_href.borrow_mut().take();
        self.map_size.borrow_mut().take();
        self.map_track_color.borrow_mut().take();
    }

    pub fn get_string(&self, k: &str) -> Option<String> {
        if let Some(d) = &self.data {
            return match k {
                "value_track_name" => Some(d.track_name.clone()),
                "value_distance" => Some(format!("{:.0}km", d.distance / 1000.0)),
                "value_speed" => Some(format!("{:.1}km/h", d.speed)),
                "value_speed_max" => Some(format!("{:.1}km/h", d.speed_max)),
                "value_speed_moving" => Some(format!("{:.1}km/h", d.speed_moving)),
                "value_uphill" => Some(format!("{:.0}m", d.uphill)),
                "value_downhill" => Some(format!("{:.0}m", d.downhill)),
                "value_elevation_max" => Some(format!("{:.0}m", d.elevation_max)),
                "value_elevation_min" => Some(format!("{:.0}m", d.elevation_min)),
                "value_time" => Some(d.time.hhmmss()),
                "value_moving_time" => Some(d.time_moving.hhmmss()),
                _ => None,
            };
        }
        None
    }

    fn build_map(&self, w_px: u32, h_px: u32, track_color: Option<galileo::Color>) -> Result<()> {
        let d = self
            .data
            .as_ref()
            .ok_or(anyhow!("error building map: missing track data"))?;

        let href = render_track_map_href(
            &d.coords,
            galileo_types::cartesian::Size::<u32>::new(w_px, h_px),
            track_color,
        )?;
        *self.map_href.borrow_mut() = Some(href);
        *self.map_size.borrow_mut() = Some((w_px, h_px));
        *self.map_track_color.borrow_mut() = track_color;

        Ok(())
    }

    pub fn get_image(
        &self,
        k: &str,
        w_px: u32,
        h_px: u32,
        track_color: Option<galileo::Color>,
    ) -> Option<String> {
        if self.data.is_some() {
            let needs_render = self.map_href.borrow().is_none()
                || self
                    .map_size
                    .borrow()
                    .is_none_or(|(cur_w, cur_h)| cur_w != w_px || cur_h != h_px)
                || *self.map_track_color.borrow() != track_color;

            if needs_render {
                self.build_map(w_px, h_px, track_color).ok()?;
            }

            return match k {
                "image_map" => self.map_href.borrow().clone(),
                _ => None,
            };
        }

        None
    }

    pub fn get_path(&self, k: &str, inp: &InputPath) -> Option<String> {
        match k {
            "path_elevation" => self.data.as_ref().map(|d| {
                let mut old_d: f64 = 0.0;
                let mut old_e: f64 = 0.0;
                let el_width = (d.elevation_max - d.elevation_min).max(100.0);
                let el_offset = d.elevation_min;
                let el_factor = inp.height / el_width;
                let s2 = d
                    .elevation
                    .iter()
                    .map(|val| ElevPoint {
                        d: val.d * (inp.length / d.distance),
                        e: (val.e - el_offset) * (el_factor),
                    })
                    .map(|val| {
                        let f = format!("l {} {}", val.d - old_d, val.e - old_e);
                        old_d = val.d;
                        old_e = val.e;
                        f
                    })
                    .collect::<Vec<String>>()
                    .join(" ");
                format!("{} {} {} l 0 {}", inp.prefix, inp.ss, s2, -old_e)
            }),
            _ => None,
        }
    }

    fn truncate_ellipsis(s: &str, max_chars: usize) -> String {
        if max_chars == 0 {
            return String::new();
        }

        let len = s.chars().count();
        if len <= max_chars {
            return s.to_string();
        }

        let take = max_chars.saturating_sub(1);
        let prefix: String = s.chars().take(take).collect();
        format!("{}…", prefix)
    }

    fn compute_track_name(gpx: &gpx::Gpx, filename: &str) -> String {
        let from_track = gpx
            .tracks
            .iter()
            .find_map(|t| t.name.as_ref())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| Context::truncate_ellipsis(s, 32));

        if let Some(name) = from_track {
            return name;
        }

        Path::new(filename)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "track".to_string())
    }

    pub fn load(&mut self) -> Result<()> {
        let file = File::open(self.filename)?;
        let reader = BufReader::new(file);

        let mut tot_distance: f64 = 0.0;
        let mut cur_distance: f64 = 0.0;
        let mut tot_time: Duration = Duration::seconds(0);
        let mut tot_moving_time: Duration = Duration::seconds(0);
        let mut uphill: f64 = 0.0;
        let mut downhill: f64 = 0.0;
        let mut speed_max: f64 = 0.0;
        let mut elevation_max: f64 = 0.0;
        let mut elevation_min: f64 = 99999.0;
        let mut elev: Vec<ElevPoint> = vec![];
        let mut coords: Vec<Point<f64>> = vec![];

        let gpx = read(reader)?;
        let track_name = Context::compute_track_name(&gpx, self.filename);
        for t in gpx.tracks {
            let mls: MultiLineString<f64> = t.multilinestring();
            tot_distance += Geodesic.length(&mls);
            for s in t.segments {
                // step by is required to filter a bit elevation variation
                s.points.iter().for_each(|f| {
                    coords.push(f.point());
                });
                let i1 = s.points.iter().step_by(10);
                let i2 = s.points.iter().step_by(10).skip(1);
                for (w1, w2) in i1.zip(i2) {
                    let p1 = w1.point();
                    let p2 = w2.point();
                    let d = Geodesic.distance(p1, p2);
                    cur_distance += d;

                    if let (Some(t1), Some(t2)) = (w1.time, w2.time) {
                        let t1: time::OffsetDateTime = t1.into();
                        let t2: time::OffsetDateTime = t2.into();
                        let ptime_time = t2 - t1;
                        let ptime = Duration::seconds(ptime_time.whole_seconds());

                        if ptime.num_seconds() > 0 {
                            tot_time += ptime;
                            let speed = (d.round() / ptime.num_seconds() as f64) * 3.6;
                            if speed > 0.5 {
                                tot_moving_time += ptime;
                            }
                            if speed > speed_max {
                                speed_max = speed;
                            }
                        }
                    }

                    if let Some(e1) = w1.elevation {
                        if let Some(e2) = w2.elevation {
                            let d = e2 - e1;
                            if d > 0.0 {
                                uphill += d;
                            } else {
                                downhill -= d;
                            }
                            if e1 > elevation_max {
                                elevation_max = e1;
                            }
                            if e1 < elevation_min {
                                elevation_min = e1;
                            }
                            elev.push(ElevPoint {
                                d: cur_distance,
                                e: e1,
                            });
                        }
                    }
                }
            }
        }

        let speed = if tot_time.num_seconds() > 0 {
            (tot_distance.round() / tot_time.num_seconds() as f64) * 3.6
        } else {
            0.0
        };
        let speed_moving = if tot_moving_time.num_seconds() > 0 {
            (tot_distance.round() / tot_moving_time.num_seconds() as f64) * 3.6
        } else {
            0.0
        };

        self.data = Some(ContextData {
            track_name,
            distance: tot_distance,
            speed,
            speed_max,
            speed_moving,
            time: tot_time,
            time_moving: tot_moving_time,
            uphill,
            downhill,
            elevation: elev,
            elevation_max,
            elevation_min,
            coords,
        });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_name_from_first_track() {
        let mut ctx = Context::new("test/t1.gpx");
        ctx.load().unwrap();
        assert_eq!(
            ctx.get_string("value_track_name").unwrap(),
            "Casalmaggiore Mountain bike"
        );
    }

    #[test]
    fn track_name_falls_back_to_filename_stem() {
        let gpx = gpx::Gpx::default();
        assert_eq!(Context::compute_track_name(&gpx, "foo_bar.gpx"), "foo_bar");
    }

    #[test]
    fn track_name_is_truncated_to_32() {
        let mut gpx = gpx::Gpx::default();
        let trk = gpx::Track {
            name: Some("abcdefghijklmnopqrstuvwxyz0123456789".to_string()),
            ..Default::default()
        };
        gpx.tracks.push(trk);

        let name = Context::compute_track_name(&gpx, "x.gpx");
        assert_eq!(name.chars().count(), 32);
        assert!(name.ends_with('…'));
    }
}
