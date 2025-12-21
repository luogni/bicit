# bicit

`bicit` generates a shareable ride summary image from a GPX track.

Pipeline:
1. Parse GPX and compute stats (distance, time, speed, elevation)
2. Render an OSM map snapshot with Galileo (cached in `.tile_cache`)
3. Inject values into an SVG template using element `id`s
4. Export SVG to PNG using Inkscape

## Requirements

- Rust toolchain (edition 2021)
- `inkscape` available on `PATH` (used for SVG → PNG export)

## Usage

Build:

```sh
cargo build --release
```

Run (example using the included sample GPX):

```sh
cargo run -- \
  --datafile test/t1.gpx \
  --template templates/dev.svg \
  --outfile out
```

Output:
- `out.svg`
- `out.png`

Notes:
- `--outfile` is a *basename*. If you pass `--outfile out.png`, it still produces `out.svg` and `out.png`.
- The map image is embedded into the generated SVG as a `data:image/png;base64,...` URL, so no extra files are left behind.

## Template placeholders

The SVG template is updated by matching element `id`s.

Text values are written into `<tspan id="...">`:
- `value_distance`
- `value_time`
- `value_moving_time`
- `value_speed`
- `value_speed_moving`
- `value_speed_max`
- `value_uphill`
- `value_downhill`
- `value_elevation_max`
- `value_elevation_min`

Graphics:
- `<path id="path_elevation" ...>`: rewritten `d` attribute for the elevation profile
- `<image id="image_map" ...>`: rewritten `xlink:href` (map)
