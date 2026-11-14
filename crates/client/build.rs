fn main() {
    println!("cargo:rerun-if-changed=assets/sticktalk.ico");
    println!("cargo:rerun-if-changed=windows.manifest");
    if std::env::var_os("CARGO_FEATURE_HEAP_PROFILE").is_some()
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        let map =
            std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("heap-profile.map");
        println!("cargo:rustc-link-arg-bin=sticktalk=/MAP:{}", map.display());
    }
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
