fn main() {
    println!("cargo:rerun-if-changed=migrations");
    println!("cargo:rerun-if-changed=static/vendor/pdfjs");
    let root =
        std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest directory"))
            .join("static/vendor/pdfjs");
    let mut files = Vec::new();
    collect(&root, &root, &mut files);
    files.sort();
    let mut source = String::from("pub static PDF_ASSETS: &[(&str, &[u8])] = &[\n");
    for (name, path) in files {
        source.push_str(&format!("({name:?}, include_bytes!({path:?})),\n"));
    }
    source.push_str("];\n");
    let output = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("output directory"));
    std::fs::write(output.join("pdf_assets.rs"), source).expect("write PDF asset manifest");
}

fn collect(root: &std::path::Path, directory: &std::path::Path, files: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(directory).expect("read PDF assets") {
        let path = entry.expect("PDF asset entry").path();
        if path.is_dir() {
            collect(root, &path, files);
        } else {
            files.push((
                path.strip_prefix(root)
                    .expect("asset under root")
                    .to_string_lossy()
                    .replace('\\', "/"),
                path.to_string_lossy().into_owned(),
            ));
        }
    }
}
