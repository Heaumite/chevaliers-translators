//! Raccourci « Chevaliers Translators » sur le bureau (0.15) : posé au premier lancement de
//! l'application, vers l'exécutable lancé. Supprimé par le joueur : il n'est pas reposé (une marque
//! dans le dossier de l'application s'en souvient). L'exécutable a changé de place (nouvelle
//! version décompressée ailleurs) et l'ancien n'existe plus : le raccourci suit le nouveau.
//! Lancé depuis un dossier temporaire (zip ouvert sans être décompressé) : pas de raccourci.

use std::path::{Path, PathBuf};

pub const NAME: &str = "Chevaliers Translators.lnk";

/// Marque du raccourci posé : chemin de l'exécutable visé.
#[cfg_attr(not(windows), allow(dead_code))]
fn marker() -> PathBuf {
    crate::fonts::cache_root().join("raccourci-bureau.txt")
}

/// Que faire du raccourci, d'après son état (logique pure, testée hors Windows).
#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    Nothing,
    /// Créer (ou recréer vers le nouvel exécutable).
    Write,
}

/// `link_exists` : le raccourci est sur le bureau ; `marked` : exécutable visé la dernière fois
/// (None : jamais posé) ; `marked_exists` : cet exécutable existe encore.
pub fn plan(exe: &Path, temp: &Path, link_exists: bool, marked: Option<&Path>, marked_exists: bool) -> Plan {
    let lower = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
    if lower(exe).starts_with(&lower(temp)) {
        return Plan::Nothing;
    }
    match (link_exists, marked) {
        // Premier lancement.
        (false, None) => Plan::Write,
        // Retiré par le joueur : on le laisse ainsi.
        (false, Some(_)) => Plan::Nothing,
        (true, Some(m)) if lower(m) != lower(exe) && !marked_exists => Plan::Write,
        (true, _) => Plan::Nothing,
    }
}

/// Bureau du joueur (suit OneDrive s'il y a déplacé son bureau).
#[cfg(windows)]
pub fn desktop() -> Option<PathBuf> {
    use windows_sys::core::PWSTR;
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_Desktop, SHGetKnownFolderPath};
    unsafe {
        let mut p: PWSTR = std::ptr::null_mut();
        let hr = SHGetKnownFolderPath(&FOLDERID_Desktop, 0, std::ptr::null_mut(), &mut p);
        if hr != 0 || p.is_null() {
            return None;
        }
        let mut n = 0;
        while *p.add(n) != 0 {
            n += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
        CoTaskMemFree(p as *const _);
        Some(PathBuf::from(s))
    }
}

/// Pose ou met à jour le raccourci si besoin ; renvoie son chemin quand il vient d'être écrit.
#[cfg(windows)]
pub fn ensure(exe: &Path) -> Result<Option<PathBuf>, String> {
    let desk = desktop().ok_or("bureau introuvable")?;
    let link = desk.join(NAME);
    let marked = std::fs::read_to_string(marker()).ok().map(|s| PathBuf::from(s.trim()));
    let marked_exists = marked.as_ref().is_some_and(|m| m.exists());
    match plan(exe, &std::env::temp_dir(), link.exists(), marked.as_deref(), marked_exists) {
        Plan::Nothing => Ok(None),
        Plan::Write => {
            write_link(&link, exe)?;
            let _ = std::fs::create_dir_all(crate::fonts::cache_root());
            let _ = std::fs::write(marker(), exe.to_string_lossy().as_bytes());
            Ok(Some(link))
        }
    }
}

/// Raccourci posé par l'application, à retirer avec elle (et sa marque).
#[cfg(windows)]
pub fn remove() -> Option<PathBuf> {
    let link = desktop()?.join(NAME);
    let _ = std::fs::remove_file(marker());
    if link.exists() && std::fs::remove_file(&link).is_ok() {
        return Some(link);
    }
    None
}

/// Écrit le fichier .lnk par l'interface de Windows (IShellLinkW, IPersistFile), sur un fil à
/// part (COM initialisé pour lui seul, sans toucher au fil de l'icône).
#[cfg(windows)]
fn write_link(link: &Path, exe: &Path) -> Result<(), String> {
    let (link, exe) = (link.to_path_buf(), exe.to_path_buf());
    std::thread::spawn(move || unsafe { com_write(&link, &exe) })
        .join()
        .map_err(|_| "écriture du raccourci interrompue".to_string())?
}

#[cfg(windows)]
unsafe fn com_write(link: &Path, exe: &Path) -> Result<(), String> {
    use std::ffi::c_void;
    use windows_sys::core::{GUID, HRESULT, PCWSTR};
    use windows_sys::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use windows_sys::Win32::UI::Shell::ShellLink;

    // Interfaces COM décrites à la main (windows-sys ne donne pas leurs tables de fonctions) :
    // ordre des méthodes de ShObjIdl_core.h et ObjIdl.h.
    type Raw = *mut c_void;
    #[repr(C)]
    struct Unknown {
        query_interface: unsafe extern "system" fn(Raw, *const GUID, *mut Raw) -> HRESULT,
        add_ref: unsafe extern "system" fn(Raw) -> u32,
        release: unsafe extern "system" fn(Raw) -> u32,
    }
    #[repr(C)]
    struct ShellLinkW {
        base: Unknown,
        get_path: usize,
        get_id_list: usize,
        set_id_list: usize,
        get_description: usize,
        set_description: unsafe extern "system" fn(Raw, PCWSTR) -> HRESULT,
        get_working_directory: usize,
        set_working_directory: unsafe extern "system" fn(Raw, PCWSTR) -> HRESULT,
        get_arguments: usize,
        set_arguments: usize,
        get_hotkey: usize,
        set_hotkey: usize,
        get_show_cmd: usize,
        set_show_cmd: usize,
        get_icon_location: usize,
        set_icon_location: unsafe extern "system" fn(Raw, PCWSTR, i32) -> HRESULT,
        set_relative_path: usize,
        resolve: usize,
        set_path: unsafe extern "system" fn(Raw, PCWSTR) -> HRESULT,
    }
    #[repr(C)]
    struct PersistFile {
        base: Unknown,
        get_class_id: usize,
        is_dirty: usize,
        load: usize,
        save: unsafe extern "system" fn(Raw, PCWSTR, i32) -> HRESULT,
        save_completed: usize,
        get_cur_file: usize,
    }
    const IID_ISHELLLINKW: GUID = GUID::from_u128(0x000214f9_0000_0000_c000_000000000046);
    const IID_IPERSISTFILE: GUID = GUID::from_u128(0x0000010b_0000_0000_c000_000000000046);
    let wide = |p: &Path| -> Vec<u16> { p.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect() };
    let text = |s: &str| -> Vec<u16> { s.encode_utf16().chain(Some(0)).collect() };

    let init = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
    let mut sl: Raw = std::ptr::null_mut();
    let hr = CoCreateInstance(&ShellLink, std::ptr::null_mut(), CLSCTX_INPROC_SERVER, &IID_ISHELLLINKW, &mut sl);
    let r = if hr < 0 || sl.is_null() {
        Err(format!("ShellLink indisponible (0x{:08X})", hr as u32))
    } else {
        let vt = &**(sl as *mut *const ShellLinkW);
        let dir = exe.parent().map(wide).unwrap_or_else(|| text(""));
        (vt.set_path)(sl, wide(exe).as_ptr());
        (vt.set_working_directory)(sl, dir.as_ptr());
        (vt.set_description)(sl, text("Traduction du chat de World of Warcraft").as_ptr());
        (vt.set_icon_location)(sl, wide(exe).as_ptr(), 0);
        let mut pf: Raw = std::ptr::null_mut();
        let hr = (vt.base.query_interface)(sl, &IID_IPERSISTFILE, &mut pf);
        let saved = if hr < 0 || pf.is_null() {
            Err(format!("IPersistFile indisponible (0x{:08X})", hr as u32))
        } else {
            let pv = &**(pf as *mut *const PersistFile);
            let hr = (pv.save)(pf, wide(link).as_ptr(), 1);
            (pv.base.release)(pf);
            if hr < 0 { Err(format!("enregistrement du raccourci impossible (0x{:08X})", hr as u32)) } else { Ok(()) }
        };
        (vt.base.release)(sl);
        saved
    };
    // S_OK ou S_FALSE (déjà initialisé dans ce mode) : à défaire ; autre mode : rien à défaire.
    if init >= 0 {
        CoUninitialize();
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_shortcut_is_placed_once_and_follows_a_moved_exe() {
        let exe = Path::new(r"D:\World of Warcraft\_retail_\Interface\AddOns\ChevaliersTranslators\ChevaliersTranslators.exe");
        let temp = Path::new(r"C:\Users\brice\AppData\Local\Temp");
        // Premier lancement : posé.
        assert_eq!(plan(exe, temp, false, None, false), Plan::Write);
        // Déjà là, vers cet exécutable : rien.
        assert_eq!(plan(exe, temp, true, Some(exe), true), Plan::Nothing);
        // Retiré par le joueur : pas reposé.
        assert_eq!(plan(exe, temp, false, Some(exe), true), Plan::Nothing);
        // Ancien exécutable disparu : le raccourci suit le nouveau.
        let old = Path::new(r"C:\Users\brice\Downloads\ct\ChevaliersTranslators.exe");
        assert_eq!(plan(exe, temp, true, Some(old), false), Plan::Write);
        // Deux copies présentes : celle du raccourci est gardée.
        assert_eq!(plan(exe, temp, true, Some(old), true), Plan::Nothing);
        // Lancé depuis un zip ouvert (dossier temporaire) : pas de raccourci.
        let tmp_exe = Path::new(r"C:\Users\brice\AppData\Local\Temp\Temp1_ct.zip\ChevaliersTranslators.exe");
        assert_eq!(plan(tmp_exe, temp, false, None, false), Plan::Nothing);
    }
}
