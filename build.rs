use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::{env, io};

use const_gen::*;
use xz2::read::XzEncoder;

fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::copy("memory.x", out.join("memory.x")).unwrap();
    println!("cargo:rustc-link-search={}", out.display());

    println!("cargo:rustc-link-arg-bins=--nmagic");
    println!("cargo:rustc-link-arg-bins=-Tlink.x");
    println!("cargo:rerun-if-changed=memory.x");

    println!("cargo:rerun-if-changed=vial.json");
    generate_vial_config(&out);
}

fn generate_vial_config(out: &Path) {
    let vial_path = Path::new("vial.json");
    let mut content = String::new();
    File::open(vial_path)
        .and_then(|mut f| f.read_to_string(&mut content).map(|_| ()))
        .unwrap_or_else(|e: io::Error| panic!("cannot read vial.json: {e}"));

    let vial_cfg = json::stringify(json::parse(&content).expect("vial.json is not valid JSON"));
    let mut compressed: Vec<u8> = Vec::new();
    XzEncoder::new(vial_cfg.as_bytes(), 6)
        .read_to_end(&mut compressed)
        .expect("failed to xz-compress vial.json");

    // Stable UID for this firmware; Vial identifies the keyboard by this.
    let keyboard_id: Vec<u8> = vec![0x56, 0x6F, 0x79, 0x61, 0x67, 0x72, 0x4D, 0x4B];

    let declarations = [
        const_declaration!(pub VIAL_KEYBOARD_DEF = compressed),
        const_declaration!(pub VIAL_KEYBOARD_ID = keyboard_id),
    ]
    .map(|s| format!("#[allow(clippy::redundant_static_lifetimes)]\n{s}"))
    .join("\n");
    fs::write(out.join("config_generated.rs"), declarations).unwrap();
}
