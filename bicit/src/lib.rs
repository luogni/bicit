pub mod context;
pub mod embedded_templates;
pub mod map;
pub mod template;

pub use context::Context;
pub use embedded_templates::{get_template_by_name, get_templates, EmbeddedTemplate};
pub use template::Template;

use anyhow::Result;

pub struct InputPath<'a> {
    pub height: f64,
    pub length: f64,
    pub ss: &'a str,
    pub prefix: &'a str,
}

impl InputPath<'_> {
    pub fn new(path: &str) -> Result<InputPath<'_>> {
        let mut ss: &str = "";
        let mut prefix: &str = "";
        let mut height: f64 = 0.0;
        let mut length: f64 = 0.0;
        let mut sx: f64 = 0.0;
        let mut sy: f64 = 0.0;

        for (i, s) in path.split(' ').enumerate() {
            if i == 0 {
                assert!(s == "m" || s == "M");
                prefix = s;
            } else if i == 1 {
                ss = s;
                let it: Vec<&str> = s.split(',').collect();
                sy = it[1].parse().unwrap();
                sx = it[0].parse().unwrap();
            } else if i == 2 {
                let it: Vec<&str> = s.split(',').collect();
                height = match prefix {
                    "m" => it[1].parse::<f64>().unwrap(),
                    "M" => it[1].parse::<f64>().unwrap() - sy,
                    _ => 0.0,
                };
                length = match prefix {
                    "m" => it[0].parse::<f64>().unwrap(),
                    "M" => it[0].parse::<f64>().unwrap() - sx,
                    _ => 0.0,
                };
            } else {
                break;
            }
        }

        Ok(InputPath {
            height,
            ss,
            length,
            prefix,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_approx_eq::assert_approx_eq;

    #[test]
    fn parse_path_m() {
        let i = InputPath::new("m 2.64583,169.33333 10.583337,-31.75").unwrap();
        assert_approx_eq!(i.height, -31.75);
        assert_approx_eq!(i.length, 10.583337);
        assert_eq!(i.ss, "2.64583,169.33333");
        assert_eq!(i.prefix, "m");
    }

    #[test]
    fn parse_path_big_m() {
        let i = InputPath::new("M 5.2,174.6 137.5,140.2").unwrap();
        assert_approx_eq!(i.height, -34.4);
        assert_approx_eq!(i.length, 132.3);
        assert_eq!(i.prefix, "M");
        assert_eq!(i.ss, "5.2,174.6");
    }
}
