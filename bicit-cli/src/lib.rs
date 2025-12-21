use anyhow::Result;
use bicit::context::Context;
use galileo::Color;
use quick_xml::Reader;
use quick_xml::Writer;
use quick_xml::events::{BytesStart, BytesText, Event};
use quick_xml::name::QName;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Copy, Clone)]
struct SvgMetrics {
    svg_px_w: f64,
    svg_px_h: f64,
    viewbox_w: f64,
    viewbox_h: f64,
}

fn parse_svg_length_to_px(s: &str) -> Option<f64> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Split numeric part and unit suffix.
    let mut split_at = 0;
    for (i, ch) in trimmed.char_indices() {
        if ch.is_ascii_digit() || ch == '.' || ch == '-' {
            split_at = i + ch.len_utf8();
        } else {
            break;
        }
    }

    if split_at == 0 {
        return None;
    }

    let value: f64 = trimmed[..split_at].parse().ok()?;
    let unit = trimmed[split_at..].trim();

    // Per SVG/CSS: unitless == px.
    let px = match unit {
        "" | "px" => value,
        "mm" => value * 96.0 / 25.4,
        "cm" => value * 96.0 / 2.54,
        "in" => value * 96.0,
        "pt" => value * 96.0 / 72.0,
        _ => return None,
    };

    Some(px)
}

fn parse_viewbox(s: &str) -> Option<(f64, f64, f64, f64)> {
    let parts: Vec<_> = s
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() != 4 {
        return None;
    }

    Some((
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
        parts[3].parse().ok()?,
    ))
}

fn image_pixels(metrics: &SvgMetrics, image_w_units: f64, image_h_units: f64) -> (u32, u32) {
    let scale_x = metrics.svg_px_w / metrics.viewbox_w;
    let scale_y = metrics.svg_px_h / metrics.viewbox_h;

    let w = (image_w_units * scale_x).round().max(1.0) as u32;
    let h = (image_h_units * scale_y).round().max(1.0) as u32;
    (w, h)
}

pub struct Template<'a> {
    filename: &'a str,
}

impl Template<'_> {
    pub fn new(filename: &str) -> Template<'_> {
        Template { filename }
    }

    pub fn apply_context_to_file(&self, context: &Context, outfile: String) -> Result<()> {
        let svg = self.apply_context(context)?;

        let outfile = PathBuf::from(outfile);
        let outbase = if outfile.extension().is_some() {
            outfile.with_extension("")
        } else {
            outfile
        };

        let outpng = outbase.with_extension("png");
        let outsvg = outbase.with_extension("svg");
        fs::write(&outsvg, svg)?;

        let inkscape_result = Command::new("inkscape")
            .arg(format!("--export-filename={}", outpng.display()))
            .arg(&outsvg)
            .output();

        context.cleanup_temp_files();
        inkscape_result?;

        Ok(())
    }

    fn apply_context(&self, context: &Context) -> Result<String> {
        let s = fs::read_to_string(self.filename)?;
        Ok(Template::apply_context_xml(&s, context))
    }

    fn get_attribute(e: &BytesStart, name: &[u8]) -> Option<String> {
        e.attributes()
            .filter_map(|x| x.ok())
            .find(|a| a.key == QName(name))
            .and_then(|attr| attr.unescape_value().ok())
            .map(|value| value.into_owned())
    }

    fn extract_hex_stroke_from_style(style: &str) -> Option<String> {
        // Very small parser: looks for "stroke:#RRGGBB" or "stroke:#RRGGBBAA".
        let needle = "stroke:";
        let idx = style.find(needle)?;
        let after = &style[idx + needle.len()..];
        let after = after.trim_start();
        let after = after.strip_prefix('#')?;

        // Extract 6 or 8 hex chars.
        let mut hex_len = 0;
        for ch in after.chars() {
            if ch.is_ascii_hexdigit() {
                hex_len += 1;
            } else {
                break;
            }
        }

        let hex_len = match hex_len {
            6 | 8 => hex_len,
            _ => return None,
        };

        Some(format!("#{}", &after[..hex_len]))
    }

    fn extract_track_color_from_template(xml: &str) -> Option<Color> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text_start = true;
        reader.config_mut().trim_text_end = true;

        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                    let Some(id) = Template::get_attribute(&e, b"id") else {
                        continue;
                    };
                    if id != "path_elevation" {
                        continue;
                    }

                    if let Some(stroke) = Template::get_attribute(&e, b"stroke")
                        && stroke.starts_with('#')
                        && let Some(color) = Color::try_from_hex(&stroke)
                    {
                        return Some(color);
                    }

                    if let Some(style) = Template::get_attribute(&e, b"style")
                        && let Some(stroke) = Template::extract_hex_stroke_from_style(&style)
                        && let Some(color) = Color::try_from_hex(&stroke)
                    {
                        return Some(color);
                    }
                }
                Ok(Event::Eof) => break,
                Ok(_) => {}
                Err(_) => break,
            }
        }

        None
    }

    fn handle_xml(
        e: &BytesStart,
        context: &Context,
        metrics: Option<&SvgMetrics>,
        track_color: Option<Color>,
    ) -> Option<String> {
        let id_t = Template::get_attribute(e, b"id");
        if let Some(id) = id_t {
            match e.name().as_ref() {
                b"tspan" => {
                    if let Some(v) = context.get_string(&id) {
                        return Some(v);
                    }
                }
                b"path" => {
                    let pathd = Template::get_attribute(e, b"d");
                    if let Some(pathd_t) = pathd {
                        let inp = bicit::InputPath::new(&pathd_t).unwrap();
                        if let Some(v) = context.get_path(&id, &inp) {
                            return Some(v);
                        }
                    }
                }
                b"image" => {
                    let wd = Template::get_attribute(e, b"width")?;
                    let hd = Template::get_attribute(e, b"height")?;
                    let w_units: f64 = wd.parse().ok()?;
                    let h_units: f64 = hd.parse().ok()?;

                    // Render the embedded bitmap at the pixel size it will be displayed at
                    // in the final exported PNG. This avoids resampling blur on map labels.
                    let (w_px, h_px) = match metrics {
                        Some(m) => image_pixels(m, w_units, h_units),
                        None => {
                            // Fallback: preserve aspect ratio.
                            let aspect = (w_units / h_units).max(0.0001);
                            let w_px = 1000u32;
                            let h_px = ((w_px as f64) / aspect).round().max(1.0) as u32;
                            (w_px, h_px)
                        }
                    };

                    let color_override = if id == "image_map" { track_color } else { None };

                    if let Some(v) = context.get_image(&id, w_px, h_px, color_override) {
                        return Some(v);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn apply_context_xml(xml: &str, context: &Context) -> String {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text_start = true;
        reader.config_mut().trim_text_end = true;

        let track_color = Template::extract_track_color_from_template(xml);

        let mut writer = Writer::new(Cursor::new(Vec::new()));
        let mut change_text: Option<String> = None;
        let mut svg_metrics: Option<SvgMetrics> = None;

        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) if e.name() == QName(b"svg") => {
                    if svg_metrics.is_none() {
                        let svg_w = Template::get_attribute(&e, b"width");
                        let svg_h = Template::get_attribute(&e, b"height");
                        let view_box = Template::get_attribute(&e, b"viewBox");

                        if let (Some(svg_w), Some(svg_h)) = (svg_w, svg_h) {
                            let svg_px_w = parse_svg_length_to_px(&svg_w);
                            let svg_px_h = parse_svg_length_to_px(&svg_h);

                            // If viewBox is missing, user-space is already px-sized.
                            let vb = view_box.as_deref().and_then(parse_viewbox).unwrap_or((
                                0.0,
                                0.0,
                                svg_px_w.unwrap_or(1.0),
                                svg_px_h.unwrap_or(1.0),
                            ));

                            if let (Some(svg_px_w), Some(svg_px_h)) = (svg_px_w, svg_px_h) {
                                svg_metrics = Some(SvgMetrics {
                                    svg_px_w,
                                    svg_px_h,
                                    viewbox_w: vb.2,
                                    viewbox_h: vb.3,
                                });
                            }
                        }
                    }

                    writer.write_event(Event::Start(e.to_owned())).unwrap();
                }
                Ok(Event::Start(e)) if e.name() == QName(b"tspan") => {
                    change_text =
                        Template::handle_xml(&e, context, svg_metrics.as_ref(), track_color);
                    writer.write_event(Event::Start(e.to_owned())).unwrap();
                }
                Ok(Event::Empty(e)) if e.name() == QName(b"path") => {
                    let pd = Template::handle_xml(&e, context, svg_metrics.as_ref(), track_color);
                    if let Some(pd) = pd {
                        let mut elem = BytesStart::new("path");
                        elem.extend_attributes(
                            e.attributes()
                                .filter_map(|attr| attr.ok())
                                .filter(|attr| attr.key != QName(b"d")),
                        );
                        elem.push_attribute(("d", pd.as_str()));
                        writer.write_event(Event::Empty(elem)).unwrap();
                    } else {
                        writer.write_event(Event::Empty(e.to_owned())).unwrap();
                    }
                }
                Ok(Event::Empty(e)) if e.name() == QName(b"image") => {
                    let pd = Template::handle_xml(&e, context, svg_metrics.as_ref(), track_color);
                    if let Some(pd) = pd {
                        let mut elem = BytesStart::new("image");
                        elem.extend_attributes(e.attributes().filter_map(|attr| attr.ok()).filter(
                            |attr| {
                                (attr.key != QName(b"xlink:href"))
                                    && (attr.key != QName(b"sodipodi:absref"))
                            },
                        ));
                        elem.push_attribute(("xlink:href", pd.as_str()));
                        writer.write_event(Event::Empty(elem)).unwrap();
                    } else {
                        writer.write_event(Event::Empty(e.to_owned())).unwrap();
                    }
                }
                Ok(Event::Text(e)) => {
                    let event = match change_text.take() {
                        Some(s) => Event::Text(BytesText::new(&s).into_owned()),
                        None => Event::Text(e.into_owned()),
                    };
                    writer.write_event(event).unwrap();
                }
                Ok(Event::Eof) => break,
                Ok(e) => writer.write_event(e.into_owned()).unwrap(),
                Err(e) => panic!("Error at position {}: {:?}", reader.buffer_position(), e),
            }
        }

        String::from_utf8(writer.into_inner().into_inner()).expect("invalid UTF8")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_approx_eq::assert_approx_eq;

    #[test]
    fn svg_length_to_px() {
        assert_approx_eq!(parse_svg_length_to_px("1080").unwrap(), 1080.0);
        assert_approx_eq!(parse_svg_length_to_px("1080px").unwrap(), 1080.0);
        assert_approx_eq!(parse_svg_length_to_px("25.4mm").unwrap(), 96.0);
        assert_approx_eq!(parse_svg_length_to_px("2.54cm").unwrap(), 96.0);
        assert_approx_eq!(parse_svg_length_to_px("1in").unwrap(), 96.0);
        assert_approx_eq!(parse_svg_length_to_px("72pt").unwrap(), 96.0);
    }

    #[test]
    fn image_pixels_for_dev_template_units() {
        // dev.svg uses a mm-like viewBox (285.75) with a px viewport (1080).
        let metrics = SvgMetrics {
            svg_px_w: 1080.0,
            svg_px_h: 1080.0,
            viewbox_w: 285.75,
            viewbox_h: 285.75,
        };

        let (w_px, h_px) = image_pixels(&metrics, 158.75, 190.5);
        assert_eq!((w_px, h_px), (600, 720));
    }

    #[test]
    fn extract_track_color_from_stroke_attr() {
        let svg = r#"<svg width='1080' height='1080' viewBox='0 0 285.75 285.75'>
  <path id='path_elevation' d='M0 0' stroke='#22C55E' />
  <image id='image_map' width='10' height='10'/>
</svg>"#;

        let c = Template::extract_track_color_from_template(svg).unwrap();
        assert_eq!(c, Color::try_from_hex("#22C55E").unwrap());
    }

    #[test]
    fn extract_track_color_from_style_attr() {
        let svg = r#"<svg width='1080' height='1080' viewBox='0 0 285.75 285.75'>
  <image id='image_map' width='10' height='10'/>
  <path id='path_elevation' d='M0 0' style='fill:none;stroke:#2db192;stroke-width:1'/>
</svg>"#;

        let c = Template::extract_track_color_from_template(svg).unwrap();
        assert_eq!(c, Color::try_from_hex("#2DB192").unwrap());
    }

    #[test]
    fn parse_xml_1() {
        let xml = r#"    <text
       xml:space="preserve"
       style="font-style:normal;font-weight:normal;font-size:10.5833px;line-height:1.25;font-family:sans-serif;letter-spacing:0px;word-spacing:0px;fill:#000000;fill-opacity:1;stroke:none;str
oke-width:0.264583"
       x="153.52859"
       y="28.334532"
       id="text_distanza"><tspan
         sodipodi:role="line"
         id="value_distance"
         x="153.52859"
         y="28.334532"
         style="stroke-width:0.264583">132</tspan></text>
"#;
        let mut context = Context::new("test/t1.gpx");
        context.load().unwrap();
        let result = Template::apply_context_xml(xml, &context);
        let expected = r#"<text
       xml:space="preserve"
       style="font-style:normal;font-weight:normal;font-size:10.5833px;line-height:1.25;font-family:sans-serif;letter-spacing:0px;word-spacing:0px;fill:#000000;fill-opacity:1;stroke:none;str
oke-width:0.264583"
       x="153.52859"
       y="28.334532"
       id="text_distanza"><tspan
         sodipodi:role="line"
         id="value_distance"
         x="153.52859"
         y="28.334532"
         style="stroke-width:0.264583">22km</tspan></text>"#;
        assert_eq!(result, expected);
    }

    #[test]
    fn parse_xml_empty() {
        let xml = r#"<tspan>TEST</tspan><tspan id="test">TEST</tspan>"#;
        let exp = r#"<tspan>TEST</tspan><tspan id="test">TEST</tspan>"#;

        let mut context = Context::new("test/t1.gpx");
        context.load().unwrap();
        let result = Template::apply_context_xml(xml, &context);
        assert_eq!(result, exp);
    }
}
