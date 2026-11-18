fn main() {
    // Embed the manifest (per-monitor DPI v2, Win10/11 compatibility) and
    // version info. Only possible when building on Windows with the resource
    // compiler available; cross-checks from other hosts skip it.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" || !cfg!(windows) {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_manifest_file("assets/nitor.manifest");
    if std::path::Path::new("assets/nitor.ico").exists() {
        res.set_icon("assets/nitor.ico");
    }
    res.set("FileDescription", "Nitor brightness control");
    res.set("ProductName", "Nitor");
    res.set("LegalCopyright", "GPL-3.0-or-later");
    res.compile().expect("failed to compile Windows resources");
    println!("cargo:rerun-if-changed=assets/nitor.manifest");
    println!("cargo:rerun-if-changed=assets/nitor.ico");
}
