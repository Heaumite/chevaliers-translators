// Ressources des exécutables (Windows, éditeur de liens MSVC) : icône (media/logo.tga réduit aux
// tailles d'icônes de Windows) et informations de version avec l'éditeur « Chevaliers » (0.14.2),
// rangées dans un fichier de ressources écrit ici, sans outil externe, un par exécutable.
// Ailleurs (Linux, éditeur de liens GNU) : rien, l'icône près de l'horloge garde son dessin de
// secours.

#[allow(dead_code)]
mod res_icone {
    include!("src/res_icone.rs");
}

fn main() {
    let manifest = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let logo = manifest.join("..").join("media").join("logo.tga");
    println!("cargo:rerun-if-changed={}", logo.display());
    println!("cargo:rerun-if-changed=src/res_icone.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if os != "windows" || env != "msvc" {
        return;
    }
    let num = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<u16>().ok()).unwrap_or(0);
    let numbers = [num("CARGO_PKG_VERSION_MAJOR"), num("CARGO_PKG_VERSION_MINOR"), num("CARGO_PKG_VERSION_PATCH"), 0];
    // Version en quatre nombres, la même en texte et en nombres (vérifiée par SignPath).
    let text_version = format!("{}.{}.{}.0", numbers[0], numbers[1], numbers[2]);
    let version = text_version.as_str();
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default());
    let tga = match std::fs::read(&logo) {
        Ok(t) => t,
        Err(e) => {
            println!("cargo:warning=icône et version de l'application non intégrées : {} : {e}", logo.display());
            return;
        }
    };
    for (bin, description, file) in [
        ("ChevaliersTranslators", "Chevaliers Translators", "ChevaliersTranslators.exe"),
        ("ct-outils", "Chevaliers Translators - outils de développement", "ct-outils.exe"),
    ] {
        let strings = res_icone::project_strings(version, description, bin, file);
        let out = out_dir.join(format!("{bin}.res"));
        let made = res_icone::app_res(&tga, &res_icone::VersionInfo { version: numbers, strings: &strings })
            .and_then(|res| std::fs::write(&out, res).map_err(|e| e.to_string()));
        match made {
            // L'éditeur de liens MSVC reconnaît un fichier .res à son contenu.
            Ok(()) => println!("cargo:rustc-link-arg-bin={bin}={}", out.display()),
            Err(e) => println!("cargo:warning=icône et version de {file} non intégrées : {e}"),
        }
    }
}
