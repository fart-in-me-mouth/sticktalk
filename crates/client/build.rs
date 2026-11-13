fn main() {
    println!("cargo:rerun-if-changed=assets/sticktalk.ico");
    println!("cargo:rerun-if-changed=windows.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/sticktalk.ico")
            .set_manifest_file("windows.manifest")
            .set("ProductName", "StickTalk")
            .set("FileDescription", "StickTalk desktop")
            .compile()
            .expect("compile Windows application resources");
    }
}
