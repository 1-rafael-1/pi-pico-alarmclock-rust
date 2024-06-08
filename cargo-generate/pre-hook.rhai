file::rename("cargo-generate/Cargo.toml", "Cargo.toml");
file::rename("cargo-generate/launch.json", ".vscode/launch.json");
let picow = variable::get("cyw43");
if picow {
    file::rename("cargo-generate/wifi-blinky.rs", "src/main.rs");
    file::rename("cargo-generate/cyw43-firmware", "cyw43-firmware");
}
file::delete("cargo-generate");
