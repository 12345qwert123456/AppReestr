//! Compile the translations (po/*.po) for running from the build tree.
//! Installed builds use the catalogs that `make install` puts into /usr/share/locale.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let po_dir = root.join("po");
    println!("cargo:rerun-if-changed={}", po_dir.display());
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("locale");
    let Ok(langs) = std::fs::read_to_string(po_dir.join("LINGUAS")) else { return };
    for lang in langs.split_whitespace() {
        let po = po_dir.join(format!("{lang}.po"));
        println!("cargo:rerun-if-changed={}", po.display());
        let dir = out.join(lang).join("LC_MESSAGES");
        std::fs::create_dir_all(&dir).unwrap();
        let ok =
            Command::new("msgfmt").arg("-o").arg(dir.join("appreestr.mo")).arg(&po).status().is_ok_and(|s| s.success());
        if !ok {
            println!("cargo:warning=msgfmt failed or is missing; {lang} translation not built");
        }
    }
    println!("cargo:rustc-env=APPREESTR_DEV_LOCALEDIR={}", out.display());
}
