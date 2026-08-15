//! Compile les ressources Windows : manifeste PerMonitorV2 et icône.

fn main() {
    println!("cargo:rerun-if-changed=res/app.manifest");
    println!("cargo:rerun-if-changed=res/compono.rc");
    println!("cargo:rerun-if-changed=res/compono.ico");
    // Le manifeste demande requireAdministrator, nécessaire pour placer des
    // fenêtres elles-mêmes élevées. `cargo test` doit donc être lancé depuis
    // un terminal déjà élevé, sans quoi le binaire de test refuse de s'exécuter.
    embed_resource::compile("res/compono.rc", embed_resource::NONE);
}
