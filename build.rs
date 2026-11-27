// Dessine l'icône CyberX et l'intègre dans cyberx.exe (Explorateur, barre des tâches, fenêtre).
include!("build_icon.rs");

fn main() {
    println!("cargo:rerun-if-changed=build_icon.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("cyberx.ico");
    std::fs::write(&out, ico(&[16, 24, 32, 48, 64, 128, 256])).expect("écriture de l'icône");
    let mut res = winresource::WindowsResource::new();
    // Identifiant 1 : utilisé par l'Explorateur et repris pour l'icône de la fenêtre.
    res.set_icon_with_id(out.to_str().unwrap(), "1");
    res.set("ProductName", "CyberX");
    res.set("FileDescription", "CyberX");
    if let Err(e) = res.compile() {
        println!("cargo:warning=Icône non intégrée à l'exécutable : {e}");
    }
}
