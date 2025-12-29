use std::fs;
use std::path::Path;

fn main() {
    let templates_dir = Path::new("templates");
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("embedded_templates.rs");

    let mut entries: Vec<_> = fs::read_dir(templates_dir)
        .expect("templates directory should exist")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().is_some_and(|f| {
                f.to_str()
                    .is_some_and(|name| name.starts_with("dev") == false)
            })
        })
        .filter(|p| p.extension().is_some_and(|ext| ext == "svg"))
        .collect();

    // Sort for deterministic output
    entries.sort();

    let mut code = String::from(
        "/// Auto-generated list of embedded templates\npub const EMBEDDED_TEMPLATES: &[EmbeddedTemplate] = &[\n",
    );

    for path in &entries {
        let name = path.file_stem().unwrap().to_string_lossy();
        let rel_path = path.strip_prefix(".").unwrap_or(path);
        code.push_str("    EmbeddedTemplate {\n");
        code.push_str(&format!("        name: \"{}\",\n", name));
        code.push_str(&format!(
            "        content: include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/{}\")),\n",
            rel_path.display()
        ));
        code.push_str("    },\n");
    }

    code.push_str("];\n");

    fs::write(&dest_path, code).unwrap();

    println!("cargo:rerun-if-changed=templates");
}
