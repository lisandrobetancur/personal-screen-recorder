fn main() {
    // ScreenCaptureKit (vía puente Swift) necesita el runtime de Swift del
    // sistema; las rutas de enlace de una dependencia no llegan al binario final.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    }
    tauri_build::build()
}
