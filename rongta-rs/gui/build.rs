//! Embeds the app icon when the packaging supplies one. The icons are rendered
//! from `assets/icon.svg` outside Cargo, so a plain `cargo build` has none and
//! the app falls back to the platform's default icon.

use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo::rustc-check-cfg=cfg(rongta_icon)");
    println!("cargo::rerun-if-env-changed=RONGTA_ICON_PNG");
    println!("cargo::rerun-if-env-changed=RONGTA_ICON_ICO");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));

    // `include_bytes!` needs a literal path, so the PNG is copied to a fixed
    // name under OUT_DIR and the cfg tells main.rs it is there.
    if let Some(png) = env::var_os("RONGTA_ICON_PNG") {
        println!("cargo::rerun-if-changed={}", PathBuf::from(&png).display());
        // fs::copy would carry over a Nix store file's read-only mode and
        // make the next run's copy fail.
        let bytes = fs::read(&png).expect("read RONGTA_ICON_PNG");
        fs::write(out_dir.join("icon.png"), bytes).expect("write icon.png");
        println!("cargo::rustc-cfg=rongta_icon");
    }

    // Explorer and the taskbar read the executable's first icon resource; the
    // window icon set at run time doesn't reach them.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && let Some(ico) = env::var_os("RONGTA_ICON_ICO")
    {
        let ico = PathBuf::from(ico);
        println!("cargo::rerun-if-changed={}", ico.display());
        let rc = out_dir.join("icon.rc");
        // Resource scripts escape a backslash as `\\` and a quote as `""`.
        let path = ico
            .display()
            .to_string()
            .replace('\\', "\\\\")
            .replace('"', "\"\"");
        fs::write(&rc, format!("1 ICON \"{path}\"\n")).expect("write icon.rc");
        embed_resource::compile(&rc, embed_resource::NONE)
            .manifest_required()
            .expect("compile the icon resource");
    }
}
