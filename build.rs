fn main() {
    // Release version (dist/package.ps1 -Release): separate from the dev
    // version in Cargo.toml; empty for dev builds.
    let release = std::env::var("DV_RELEASE").unwrap_or_default();
    println!("cargo:rustc-env=DV_RELEASE={release}");
    println!("cargo:rerun-if-env-changed=DV_RELEASE");
    // Embed the app icon and version info into the Windows executable.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "Drifting VOID");
        res.set("FileDescription", "Drifting VOID");
        if !release.is_empty() {
            res.set("ProductVersion", &release);
        }
        res.compile().expect("embedding the Windows icon");
    }
    println!("cargo:rerun-if-changed=assets/icon.ico");
}
