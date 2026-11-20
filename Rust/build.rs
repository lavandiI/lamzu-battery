//! Build script: embeds the application icon, version information and manifest.
//!
//! The resource step needs the Windows SDK (rc.exe/link.exe); when it is missing
//! the build still succeeds and only loses the shell integration.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/app.manifest");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("assets/icon.ico")
        .set_manifest_file("assets/app.manifest")
        .set("ProductName", "LAMZU Battery")
        .set(
            "FileDescription",
            "LAMZU Battery - battery indicator for LAMZU mice",
        )
        .set("OriginalFilename", "LamzuBattery.exe")
        .set("LegalCopyright", "MIT License")
        .set("FileVersion", &format!("{version}.0"))
        .set("ProductVersion", &version)
        .set_language(0x0409);

    if let Err(error) = resource.compile() {
        println!("cargo:warning=could not embed Windows resources: {error}");
    }
}
