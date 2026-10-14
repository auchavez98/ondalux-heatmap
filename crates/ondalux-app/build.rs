//! Embeds the logo as the Windows executable icon (Explorer, desktop
//! shortcuts, taskbar). The icon is generated from `ondalux-report/src/logo.rs`.

#[path = "../ondalux-report/src/logo.rs"]
#[allow(dead_code)]
mod logo;

fn main() {
    println!("cargo:rerun-if-changed=../ondalux-report/src/logo.rs");
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
        let ico = out.join("ondalux.ico");
        std::fs::write(&ico, logo::ico()).expect("write icon");
        let mut res = winresource::WindowsResource::new();
        res.set_icon(&ico.to_string_lossy());
        res.set("ProductName", "Ondalux Heatmap");
        res.set("FileDescription", "Ondalux Heatmap");
        res.compile().expect("compile Windows resources");
    }
}
