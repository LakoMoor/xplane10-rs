//! Compiles the C shim against the system FreeType found by pkg-config.
fn main() {
    let ft = pkg_config::Config::new()
        .probe("freetype2")
        .expect("FreeType development files (pkg-config freetype2) are required for UI text");
    let mut build = cc::Build::new();
    build.file("src/shim.c");
    for p in &ft.include_paths {
        build.include(p);
    }
    build.compile("xpft_shim");
    println!("cargo:rerun-if-changed=src/shim.c");
}
