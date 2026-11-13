//! Touches simulées pour l'envoi traduit, avec l'accord du joueur (09/11/2026) : après son
//! Maj+Entrée, et seulement quand l'addon signale que la boîte de saisie de WoW est rouverte et
//! vide, la traduction y est collée (Ctrl+V) puis envoyée (Entrée). Un appui du joueur, un
//! message envoyé. Rien d'autre n'est jamais simulé.

use std::time::Duration;

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    MAPVK_VK_TO_VSC, VK_CONTROL, VK_LSHIFT, VK_RETURN, VK_RSHIFT, VK_SHIFT, VK_V,
};

fn key(vk: u16, up: bool) -> INPUT {
    // Code de touche matériel en plus du code virtuel : WoW lit l'un ou l'autre selon la touche.
    let scan = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) } as u16;
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: if up { KEYEVENTF_KEYUP } else { 0 }, time: 0, dwExtraInfo: 0 },
        },
    }
}

fn send(keys: &[INPUT]) -> Result<(), String> {
    let n = unsafe { SendInput(keys.len() as u32, keys.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
    if n as usize == keys.len() {
        Ok(())
    } else {
        Err(format!("SendInput : {n} touches passées sur {}", keys.len()))
    }
}

/// Le joueur tient-il Maj enfoncée ?
pub fn shift_held() -> bool {
    unsafe { (GetAsyncKeyState(VK_SHIFT as i32) as u16 & 0x8000) != 0 }
}

/// Le joueur tient-il encore Maj ou Entrée (0.16.4) ? Entrée tenue se répète dans le jeu et
/// refermait la boîte rouverte avant le collage : la traduction partait dans le vide.
pub fn keys_held() -> bool {
    shift_held() || unsafe { (GetAsyncKeyState(VK_RETURN as i32) as u16 & 0x8000) != 0 }
}

/// Ctrl+V puis Entrée dans la fenêtre active (WoW). Maj encore tenue : relâchée d'abord, sinon
/// le jeu verrait Maj+Entrée.
pub fn paste_and_enter() -> Result<(), String> {
    let mut keys = Vec::new();
    if shift_held() {
        keys.extend([key(VK_LSHIFT, true), key(VK_RSHIFT, true), key(VK_SHIFT, true)]);
    }
    keys.extend([key(VK_CONTROL, false), key(VK_V, false), key(VK_V, true), key(VK_CONTROL, true)]);
    send(&keys)?;
    // Le collage est traité par le jeu à l'image suivante : Entrée un peu après.
    std::thread::sleep(Duration::from_millis(60));
    send(&[key(VK_RETURN, false), key(VK_RETURN, true)])
}
