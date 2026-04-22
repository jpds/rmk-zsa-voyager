//! Vial keyboard-definition blob generated at build time from `vial.json`.
//! See `build.rs::generate_vial_config`.

include!(concat!(env!("OUT_DIR"), "/config_generated.rs"));
