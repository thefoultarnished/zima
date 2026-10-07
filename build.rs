fn main() {
    slint_build::compile("ui/app.slint").unwrap();

    // The icon Explorer, the Start menu and the installer show for zima.exe, plus its version
    // (taken from Cargo.toml), which the installer reads.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=ui/zima.ico");
        winresource::WindowsResource::new().set_icon("ui/zima.ico").compile().unwrap();
    }
}
