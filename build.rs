//! Compile les ressources Windows : manifeste PerMonitorV2 et icône.

fn main() {
    println!("cargo:rerun-if-changed=res/app.manifest");
    println!("cargo:rerun-if-changed=res/compono.rc");
    println!("cargo:rerun-if-changed=res/compono.ico");
    embed_resource::compile("res/compono.rc", embed_resource::NONE);
}
