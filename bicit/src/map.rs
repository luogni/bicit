use std::io::Cursor;
use std::time::Duration;

use anyhow::{Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use galileo::layer::feature_layer::symbol::Symbol;
use galileo::layer::raster_tile_layer::RasterTileLayerBuilder;
use galileo::layer::{FeatureLayer, feature_layer::FeatureLayerOptions};
use galileo::render::render_bundle::RenderBundle;
use galileo::render::{LineCap, LinePaint, WgpuRenderer};
use galileo::tile_schema::TileSchemaBuilder;
use galileo::{Color, Map, MapView, Messenger};
use galileo_types::cartesian::{CartesianPoint3d, Point3, Size};
use galileo_types::geo::impls::GeoPoint2d;
use galileo_types::geo::{Crs, NewGeoPoint};
use galileo_types::geometry::Geom;
use galileo_types::geometry_type::GeoSpace2d;
use galileo_types::impls::Contour;
use galileo_types::{Disambig, MultiContour};
use geo::Simplify;
use geo_types::{LineString, Point};
use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};

#[derive(Debug, Copy, Clone)]
struct RoundSegmentContourSymbol {
    color: Color,
    width: f64,
}

impl<F> Symbol<F> for RoundSegmentContourSymbol {
    fn render(
        &self,
        _feature: &F,
        geometry: &Geom<Point3>,
        min_resolution: f64,
        bundle: &mut RenderBundle,
    ) {
        // Galileo currently strokes lines using a miter join, which tends to produce
        // visible spikes/triangles on dense GPX tracks. Rendering as independent
        // 2-point segments avoids joins altogether.
        let paint = LinePaint {
            color: self.color,
            width: self.width,
            offset: 0.0,
            line_cap: LineCap::Round,
        };

        match geometry {
            Geom::Contour(contour) => {
                add_round_capped_segments(contour, &paint, min_resolution, bundle)
            }
            Geom::MultiContour(contours) => {
                for contour in contours.contours() {
                    add_round_capped_segments(contour, &paint, min_resolution, bundle);
                }
            }
            _ => {}
        }
    }
}

fn add_round_capped_segments<C>(
    contour: &C,
    paint: &LinePaint,
    min_resolution: f64,
    bundle: &mut RenderBundle,
) where
    C: galileo_types::contour::Contour<Point = Point3>,
{
    let mut iterator = contour.iter_points();
    let Some(first) = iterator.next() else {
        return;
    };

    let mut prev = first;
    for p in iterator {
        if p.x() == prev.x() && p.y() == prev.y() && p.z() == prev.z() {
            continue;
        }

        let segment = Contour::open(vec![prev, p]);
        bundle.add_line(&segment, paint, min_resolution);
        prev = p;
    }

    if contour.is_closed() && prev.x() != first.x()
        || prev.y() != first.y()
        || prev.z() != first.z()
    {
        let segment = Contour::open(vec![prev, first]);
        bundle.add_line(&segment, paint, min_resolution);
    }
}

fn dedupe_consecutive_coords(coords: &[Point<f64>]) -> Vec<Point<f64>> {
    let mut out: Vec<Point<f64>> = Vec::with_capacity(coords.len());
    for p in coords {
        if out
            .last()
            .is_some_and(|last| last.x() == p.x() && last.y() == p.y())
        {
            continue;
        }
        out.push(*p);
    }
    out
}

/// Renders an OSM map with the provided track overlay and returns a `data:image/png;base64,...` href.
///
/// `coords` are expected to be WGS84 lon/lat points.
pub fn render_track_map_href(
    coords: &[Point<f64>],
    image_size: Size<u32>,
    track_color: Option<Color>,
) -> Result<String> {
    if coords.is_empty() {
        return Err(anyhow!("error building map: no coordinates"));
    }

    if image_size.width() == 0 || image_size.height() == 0 {
        return Err(anyhow!("error building map: invalid image size"));
    }

    let coords = dedupe_consecutive_coords(coords);

    // Simplify a potentially very dense polyline (reduces render time / overdraw).
    let raw_line: LineString<f64> = coords.iter().map(|p| (p.x(), p.y())).collect();
    let simplified = simplify_linestring(&raw_line, 2000);

    let points: Vec<GeoPoint2d> = simplified
        .points()
        .map(|p| NewGeoPoint::latlon(p.y(), p.x()))
        .collect();

    let contour: Disambig<Contour<GeoPoint2d>, GeoSpace2d> = Disambig::new(Contour::open(points));

    // "Cased" line: outline + inner stroke.
    let track_outline_layer = FeatureLayer::new(
        vec![contour.clone()],
        RoundSegmentContourSymbol {
            color: Color::rgba(0, 0, 0, 200),
            width: 10.0,
        },
        Crs::WGS84,
    )
    .with_options(FeatureLayerOptions {
        sort_by_depth: true,
        use_antialiasing: true,
        ..Default::default()
    });

    let track_color = track_color.unwrap_or(Color::rgba(255, 45, 85, 255));

    let track_inner_layer = FeatureLayer::new(
        vec![contour],
        RoundSegmentContourSymbol {
            color: track_color,
            width: 6.0,
        },
        Crs::WGS84,
    )
    .with_options(FeatureLayerOptions {
        sort_by_depth: true,
        use_antialiasing: true,
        ..Default::default()
    });

    let extent = track_inner_layer
        .extent_projected(&Crs::EPSG3857)
        .ok_or(anyhow!("error building map: track extent unavailable"))?;
    let center = extent.center();

    let width_resolution = extent.width() / image_size.width() as f64;
    let height_resolution = extent.height() / image_size.height() as f64;
    let min_resolution = TileSchemaBuilder::web_mercator(0..=18)
        .build()
        .expect("default tile schema is valid")
        .lod_resolution(17)
        .expect("tile schema has zoom level 17");
    let resolution = (width_resolution.max(height_resolution) * 1.1).max(min_resolution);

    let mut osm = RasterTileLayerBuilder::new_osm()
        .with_file_cache_checked(".tile_cache")
        .build()
        .map_err(|e| anyhow!("error creating OSM layer: {e}"))?;
    // Without this, the first render can be partially transparent due to fade-in.
    osm.set_fade_in_duration(Duration::default());

    let map_view = MapView::new_projected(&center, resolution).with_size(image_size.cast());

    // Galileo tile loading & rendering are async; keep this module usable from sync callers.
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        osm.load_tiles(&map_view).await;
    });

    let map = Map::new(
        map_view,
        vec![
            Box::new(osm),
            Box::new(track_outline_layer),
            Box::new(track_inner_layer),
        ],
        None::<Box<dyn Messenger>>,
    );

    let renderer = runtime
        .block_on(async { WgpuRenderer::new_with_texture_rt(image_size).await })
        .ok_or(anyhow!("error creating renderer"))?;

    renderer
        .render(&map)
        .map_err(|e| anyhow!("error rendering map: {e}"))?;

    let bitmap = runtime
        .block_on(async { renderer.get_image().await })
        .map_err(|e| anyhow!("error retrieving rendered bitmap: {e}"))?;

    let buffer =
        ImageBuffer::<Rgba<u8>, _>::from_raw(image_size.width(), image_size.height(), bitmap)
            .ok_or(anyhow!("error creating image buffer"))?;

    let mut png = Vec::new();
    DynamicImage::ImageRgba8(buffer).write_to(&mut Cursor::new(&mut png), ImageFormat::Png)?;

    Ok(format!("data:image/png;base64,{}", BASE64.encode(png)))
}

fn simplify_linestring(raw: &LineString<f64>, max_points: usize) -> LineString<f64> {
    // Units are degrees; values correspond roughly to ~1–20 meters.
    let mut simplified = raw.clone();
    for epsilon in [0.0, 0.00001, 0.00003, 0.00005, 0.0001, 0.0002] {
        let candidate = raw.simplify(epsilon);
        if candidate.0.len() >= 2 {
            simplified = candidate;
        }
        if simplified.0.len() <= max_points {
            break;
        }
    }
    simplified
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplify_keeps_endpoints() {
        let raw: LineString<f64> = vec![(0.0, 0.0), (0.000001, 0.000001), (1.0, 1.0)].into();
        let simplified = simplify_linestring(&raw, 2);
        assert_eq!(simplified.0.first(), raw.0.first());
        assert_eq!(simplified.0.last(), raw.0.last());
    }

    // NOTE: no render test here (requires wgpu/GPU).
}
