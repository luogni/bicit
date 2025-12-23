/// Represents an embedded SVG template
#[derive(Debug, Clone, Copy)]
pub struct EmbeddedTemplate {
    /// Template name (filename without extension)
    pub name: &'static str,
    /// SVG content
    pub content: &'static str,
}

// Include the auto-generated template list from build.rs
include!(concat!(env!("OUT_DIR"), "/embedded_templates.rs"));

/// Get all embedded templates
pub fn get_templates() -> &'static [EmbeddedTemplate] {
    EMBEDDED_TEMPLATES
}

/// Find a template by name
pub fn get_template_by_name(name: &str) -> Option<&'static EmbeddedTemplate> {
    EMBEDDED_TEMPLATES.iter().find(|t| t.name == name)
}
