//! Presse-papiers : collage de l'envoi traduit (0.11), copie d'un message du chat (icône de
//! copie), et contenu du joueur remis après un collage.

#[cfg(windows)]
pub fn set(text: &str) -> Result<(), String> {
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE,
    };
    use windows_sys::Win32::System::Ole::CF_UNICODETEXT;

    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = wide.len() * 2;
    unsafe {
        if OpenClipboard(null_mut()) == 0 {
            return Err("presse-papiers occupé par un autre programme".into());
        }
        let result = (|| {
            if EmptyClipboard() == 0 {
                return Err("EmptyClipboard a échoué".to_string());
            }
            let mem = GlobalAlloc(GMEM_MOVEABLE, bytes);
            if mem.is_null() {
                return Err("GlobalAlloc a échoué".to_string());
            }
            let dst = GlobalLock(mem) as *mut u16;
            if dst.is_null() {
                GlobalFree(mem);
                return Err("GlobalLock a échoué".to_string());
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
            GlobalUnlock(mem);
            // Après un SetClipboardData réussi, la mémoire appartient au système.
            if SetClipboardData(CF_UNICODETEXT as u32, mem).is_null() {
                GlobalFree(mem);
                return Err("SetClipboardData a échoué".to_string());
            }
            Ok(())
        })();
        CloseClipboard();
        result
    }
}

#[cfg(not(windows))]
#[allow(dead_code)]
pub fn set(_text: &str) -> Result<(), String> {
    Err("presse-papiers Windows indisponible ici".into())
}

/// Texte du presse-papiers, s'il en contient.
#[cfg(windows)]
pub fn get() -> Option<String> {
    use std::ptr::null_mut;
    use windows_sys::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard};
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalUnlock};
    use windows_sys::Win32::System::Ole::CF_UNICODETEXT;
    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT as u32) == 0 || OpenClipboard(null_mut()) == 0 {
            return None;
        }
        let h = GetClipboardData(CF_UNICODETEXT as u32);
        let mut out = None;
        if !h.is_null() {
            let p = GlobalLock(h) as *const u16;
            if !p.is_null() {
                let mut n = 0usize;
                while n < (1 << 20) && *p.add(n) != 0 {
                    n += 1;
                }
                out = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, n)));
                GlobalUnlock(h);
            }
        }
        CloseClipboard();
        out
    }
}

#[cfg(not(windows))]
#[allow(dead_code)]
pub fn get() -> Option<String> {
    None
}
