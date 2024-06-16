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
    wifi_secrets().unwrap();
}

fn wifi_secrets() -> io::Result<()> {
    println!("in wifi_secrets");
    // Read the wifi_config.json file and write the SSID and password to wifi_secrets.rs

    // Create a new file in the output directory
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR environment variable not set");
    let dest_path = Path::new(&out_dir).join("wifi_secrets.rs");
    let mut f = File::create(&dest_path).expect("Could not create wifi_secrets.rs file");

    // Read the wifi_config.json file, or create it with dummy values if it doesn't exist
    let config_path = Path::new("wifi_config.json");
    let config_contents = if config_path.exists() {
        fs::read_to_string(config_path).expect("Could not read wifi_config.json file")
    } else {
        println!("wifi_config.json not found, creating with dummy values");
        let dummy_config = r#"{"ssid":"dummy","password":"dummy"}"#;
        fs::write(config_path, dummy_config).expect("Could not write dummy wifi_config.json file");
        dummy_config.to_string()
    };

    // Parse the JSON and extract the SSID and password
    let config: serde_json::Value =
        serde_json::from_str(&config_contents).expect("Could not parse wifi_config.json file");
    let ssid = config["ssid"]
        .as_str()
        .expect("ssid not found in wifi_config.json file");
    let password = config["password"]
        .as_str()
        .expect("password not found in wifi_config.json file");

    // Write the SSID and password to wifi_secrets.rs
    println!("in wifi_secrets, before writing ssid and password to output file");
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
