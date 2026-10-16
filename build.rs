//! Embeds the application icon and version details in Windows builds, so the
//! taskbar, Explorer, and Alt+Tab show the OpenIRC icon. Other platforms take
//! the icon from the window at run time (`ui::run`).

#[allow(dead_code)]
#[path = "src/icon_art.rs"]
mod icon_art;

fn main() {
    println!("cargo:rerun-if-changed=src/icon_art.rs");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let icon = out.join("openirc.ico");
    if let Err(err) = std::fs::write(&icon, icon_art::ico(&[16, 20, 24, 32, 40, 48, 64, 256])) {
        println!("cargo:warning=could not write the icon: {err}");
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon(&icon.to_string_lossy())
        .set("FileDescription", "OpenIRC")
        .set("ProductName", "OpenIRC");
    // A missing resource compiler (windres or rc.exe) should not stop the
    // build; the program works the same, only without the embedded icon.
    if let Err(err) = resource.compile() {
        println!("cargo:warning=the Windows icon was not embedded: {err}");
    }
}
