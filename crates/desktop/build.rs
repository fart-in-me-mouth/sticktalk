fn main() {
    // The app icon lives with the rest of the app artwork (crates/client/assets).
    println!("cargo:rerun-if-changed=../client/assets/sticktalk.ico");
    println!("cargo:rerun-if-changed=windows.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("../client/assets/sticktalk.ico")
            .set_manifest_file("windows.manifest")
            .set("ProductName", "StickTalk")
            .set("FileDescription", "StickTalk")
            .compile()
            .expect("compile Windows application resources");
    }
}
