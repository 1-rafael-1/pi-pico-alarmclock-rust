//! This build script copies the `memory.x` file from the crate root into
//! a directory where the linker can always find it at build time.
//! For many projects this is optional, as the linker always searches the
//! project root directory -- wherever `Cargo.toml` is. However, if you
//! are using a workspace or have a more complicated build setup, this
//! build script becomes required. Additionally, by requesting that
//! Cargo re-run the build script whenever `memory.x` is changed,
//! updating `memory.x` ensures a rebuild of the application with the
//! new memory settings.

use serde_json;
use std::env;
use std::fs;
use std::fs::File;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

fn main() {
    println!("in build.rs");
    memory_x();
    let _ = wifi_secrets(); // ToDo: Handle error
}

fn wifi_secrets() -> io::Result<()> {
    print!("in wifi_secrets");
    // Fetch the output directory from the OUT_DIR environment variable
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR environment variable not set");
    let dest_path = Path::new(&out_dir).join("wifi_secrets.rs");
    let mut f = File::create(&dest_path)?;

    let config_contents = fs::read_to_string("wifi_config.json").unwrap();
    let config: serde_json::Value = serde_json::from_str(&config_contents).unwrap();

    let ssid = config["ssid"].as_str().unwrap_or("default_ssid");
    let password = config["password"].as_str().unwrap_or("default_password");

    writeln!(f, "pub const SSID: &str = {:?};", ssid)?;
    writeln!(f, "pub const PASSWORD: &str = {:?};", password)?;

    Ok(())
}

fn memory_x() {
    print!("in memory_x");
    // Put `memory.x` in our output directory and ensure it's
    // on the linker search path.
    let out = &PathBuf::from(env::var_os("OUT_DIR").unwrap());
    File::create(out.join("memory.x"))
        .unwrap()
        .write_all(include_bytes!("memory.x"))
        .unwrap();
    println!("cargo:rustc-link-search={}", out.display());

    // By default, Cargo will re-run a build script whenever
    // any file in the project changes. By specifying `memory.x`
    // here, we ensure the build script is only re-run when
    // `memory.x` is changed.
    println!("cargo:rerun-if-changed=memory.x");

    println!("cargo:rustc-link-arg-bins=--nmagic");
    println!("cargo:rustc-link-arg-bins=-Tlink.x");
    println!("cargo:rustc-link-arg-bins=-Tlink-rp.x");
    println!("cargo:rustc-link-arg-bins=-Tdefmt.x");
}
