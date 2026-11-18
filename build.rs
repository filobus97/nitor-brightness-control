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
    let plus = std::env::var_os("CARGO_FEATURE_PLUS").is_some();
    let product = if plus { "Nitor Plus" } else { "Nitor" };
    res.set("FileDescription", &format!("{product} brightness control"));
    res.set("ProductName", product);
    // Nitor Plus gets the amber variant of the icon.
    let icon = if plus {
        "assets/nitor-plus.ico"
    } else {
        "assets/nitor.ico"
    };
    res.set_icon(icon);
    res.set("LegalCopyright", "GPL-3.0-or-later");
    res.compile().expect("failed to compile Windows resources");
    println!("cargo:rerun-if-changed=assets/nitor.manifest");
    println!("cargo:rerun-if-changed=assets/nitor.ico");
    println!("cargo:rerun-if-changed=assets/nitor-plus.ico");
}
