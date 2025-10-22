//! This build script copies the `memory.x` file from the crate root into
//! a directory where the linker can always find it at build time.
//! For many projects this is optional, as the linker always searches the
//! project root directory -- wherever `Cargo.toml` is. However, if you
//! are using a workspace or have a more complicated build setup, this
//! build script becomes required. Additionally, by requesting that
//! Cargo re-run the build script whenever `memory.x` is changed,
//! updating `memory.x` ensures a rebuild of the application with the
//! new memory settings.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::print_stdout)]

use std::{
    env, fs,
    fs::File,
    io,
    io::Write,
    path::{Path, PathBuf},
};

fn main() {
    println!("in build.rs");
    memory_x();
    wifi_secrets().unwrap();
    time_api_config().unwrap();
}

/// Generate `wifi_secrets.rs` from `wifi_config.json`
fn wifi_secrets() -> io::Result<()> {
    println!("in wifi_secrets");
    // Read the wifi_config.json file and write the SSID and password to wifi_secrets.rs

    // Create a new file in the output directory
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR environment variable not set");
    let dest_path = Path::new(&out_dir).join("wifi_secrets.rs");
    let mut f = File::create(dest_path).expect("Could not create wifi_secrets.rs file");

    // Read the wifi_config.json file, or create it with dummy values if it doesn't exist
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR environment variable not set");
    let config_path = Path::new(&manifest_dir).join("config/wifi_config.json");
    //let config_path = Path::new("src/config/wifi_config.json");
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
    writeln!(f, "pub const SSID: &str = \"{ssid}\";")?;
    writeln!(f, "pub const PASSWORD: &str = \"{password}\";")?;
    Ok(())
}

/// Generate `time_api_config.rs` from `time_api.json`
fn time_api_config() -> io::Result<()> {
    println!("in time_api_config");
    // Read the time_api.json file and write the URL and timezone to time_api_config.rs

    // Create a new file in the output directory
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR environment variable not set");
    let dest_path = Path::new(&out_dir).join("time_api_config.rs");
    let mut f = File::create(dest_path).expect("Could not create time_api_config.rs file");

    // Read the time_api.json file, or create it with dummy values if it doesn't exist
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR environment variable not set");
    let config_path = Path::new(&manifest_dir).join("config/time_api.json");
    //let config_path = Path::new("src/config/time_api.json");
    let config_contents = if config_path.exists() {
        fs::read_to_string(config_path).expect("Could not read time_api.json file")
    } else {
        println!("time_api.json not found, creating with dummy values");
        let dummy_config = r#"{"time api by zone":{"baseurl":"dummy","timezone":"dummy"}}"#;
        fs::write(config_path, dummy_config).expect("Could not write dummy time_api.json file");
        dummy_config.to_string()
    };

    // Parse the JSON and extract the URL and timezone
    let config: serde_json::Value = serde_json::from_str(&config_contents).expect("Could not parse time_api.json file");
    let baseurl = config["time api by zone"]["baseurl"]
        .as_str()
        .expect("baseurl not found in time_api.json file");
    let timezone = config["time api by zone"]["timezone"]
        .as_str()
        .expect("timezone not found in time_api.json file");

    // Combine baseurl and timezone into a single string for TIME_SERVER_URL
    let combined_url = format!("{baseurl}{timezone}");

    // Write the baseurl and timezone to time_api_secrets.rs
    writeln!(f, "pub const TIME_SERVER_URL: &str = \"{combined_url}\";")?;
    Ok(())
}

/// Handle the `memory.x` linker script
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
