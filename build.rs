//! Shrinks icon.png to the window icon's size at build time, so the binary carries 256 KB of
//! ready RGBA instead of the full-size PNG (and nothing has to be decoded at startup).

fn main() {
    println!("cargo:rerun-if-changed=icon.png");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("icon.rgba");
    let img = image::open("icon.png").expect("icon.png").resize_exact(256, 256, image::imageops::FilterType::Lanczos3).into_rgba8();
    std::fs::write(out, img.as_raw()).unwrap();
}
