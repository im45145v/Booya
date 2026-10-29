// The ScreenCaptureKit bridge (a Swift static library, used on macOS only)
// needs the system Swift runtime. Its own build script adds this rpath to its
// own targets only, so every crate that produces an executable or a test
// binary has to add it too, or the binary dies at launch with
// "Library not loaded: @rpath/libswift_Concurrency.dylib".
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    }
}
