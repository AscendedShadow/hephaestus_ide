fn main() {
    println!("cargo:rerun-if-changed=resources/hephaestus.rc");
    println!("cargo:rerun-if-changed=resources/hephaestus.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("resources/hephaestus.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("failed to embed the Hephaestus icon");
    }
}
