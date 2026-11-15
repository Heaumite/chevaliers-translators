//! Lancement au démarrage de Windows (0.16.6) : jamais par défaut, seulement coché par le joueur
//! dans le menu de l'icône (« Lancer au démarrage de Windows »). Une valeur dans la clé « Run »
//! de l'utilisateur (aucun droit d'administrateur) lance l'application sans fenêtre
//! (« --discret ») à l'ouverture de la session. L'exécutable a changé de place (mise à jour,
//! nouvelle décompression) : la valeur suit le nouveau. Retirée par « Désinstaller... ».

/// Nom de la valeur dans HKCU\Software\Microsoft\Windows\CurrentVersion\Run.
pub const VALUE: &str = "Chevaliers Translators";
#[cfg_attr(not(windows), allow(dead_code))]
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

/// Ligne de commande enregistrée pour un exécutable.
pub fn command_for(exe: &std::path::Path) -> String {
    format!("\"{}\" --discret", exe.display())
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Commande enregistrée (None : lancement au démarrage désactivé).
#[cfg(windows)]
pub fn current() -> Option<String> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let (key, value) = (wide(RUN_KEY), wide(VALUE));
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr() as *mut _, &mut size)
    };
    if r != 0 {
        return None;
    }
    let n = (size as usize / 2).saturating_sub(1).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..n]))
}

#[cfg(not(windows))]
pub fn current() -> Option<String> {
    None
}

pub fn enabled() -> bool {
    current().is_some()
}

/// Active (vers l'exécutable lancé) ou désactive le lancement au démarrage.
#[cfg(windows)]
pub fn set(on: bool) -> Result<(), String> {
    use windows_sys::Win32::System::Registry::{RegDeleteKeyValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
    let (key, value) = (wide(RUN_KEY), wide(VALUE));
    if !on {
        let r = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) };
        // Déjà absente (2 : introuvable) : rien à faire.
        return if r == 0 || r == 2 { Ok(()) } else { Err(format!("clé Run : erreur {r}")) };
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let data = wide(&command_for(&exe));
    let r = unsafe {
        RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), REG_SZ, data.as_ptr() as *const _, (data.len() * 2) as u32)
    };
    if r == 0 {
        Ok(())
    } else {
        Err(format!("clé Run : erreur {r}"))
    }
}

#[cfg(not(windows))]
pub fn set(_on: bool) -> Result<(), String> {
    Err("seulement sous Windows".into())
}

/// Au lancement : activé, mais vers un autre exécutable (déplacé, mis à jour ailleurs) : suit
/// celui-ci. Rien si le joueur ne l'a pas activé.
pub fn follow_exe() {
    let (Some(cmd), Ok(exe)) = (current(), std::env::current_exe()) else { return };
    if cmd != command_for(&exe) {
        match set(true) {
            Ok(()) => crate::say!("[démarrage] lancement au démarrage de Windows mis à jour vers {}", exe.display()),
            Err(e) => crate::say!("[démarrage] mise à jour du lancement au démarrage impossible : {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_is_quoted_and_silent() {
        let c = command_for(std::path::Path::new("C:\\Program Files\\CT\\ChevaliersTranslators.exe"));
        assert_eq!(c, "\"C:\\Program Files\\CT\\ChevaliersTranslators.exe\" --discret");
        // Jamais activé par défaut : rien d'enregistré hors Windows (et rien posé au lancement).
        #[cfg(not(windows))]
        assert!(!enabled());
    }
}
