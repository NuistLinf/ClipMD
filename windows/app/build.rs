//! 把图标、清单（DPI 感知、新版控件）和版本信息编进 exe。
//! 只有在 Windows 上编译 Windows 版时才需要（要用 Windows SDK 的 rc.exe）；在 macOS 上做类型检查时跳过。
fn main() {
    println!("cargo:rerun-if-changed=res/ClipMD.rc");
    println!("cargo:rerun-if-changed=res/ClipMD.manifest");
    println!("cargo:rerun-if-changed=res/ClipMD.ico");
    let target_windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    if target_windows && cfg!(windows) {
        embed_resource::compile("res/ClipMD.rc", embed_resource::NONE).manifest_required().unwrap();
    }
}
