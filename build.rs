fn main() {
    // Embed the app icon and version info into the Windows executable.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "Drifting VOID");
        res.set("FileDescription", "Drifting VOID");
        res.compile().expect("embedding the Windows icon");
    }
    println!("cargo:rerun-if-changed=assets/icon.ico");
}
