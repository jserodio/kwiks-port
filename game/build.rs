fn main() {
    // Este sistema solo trae runtime de SDL3/libopenmpt (sin ficheros de
    // desarrollo), así que enlazamos contra los symlinks de `game/lib`.
    let lib = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("lib");
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rerun-if-changed={}", lib.display());
}
