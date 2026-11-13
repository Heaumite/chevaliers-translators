//! Capture GDI du coin bas-droit de la zone de jeu de WoW (Windows uniquement), là où l'addon
//! peint son bandeau : la capture suit la fenêtre de WoW, en plein écran comme en mode
//! fenêtré (0.8.6 ; avant, elle lisait le coin de l'écran, faux en mode fenêtré). Sans fenêtre
//! de WoW, le coin bas-droit de l'écran principal.

use std::ffi::c_void;
use std::ptr::null_mut;

use windows_sys::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush, GetDC,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC,
    HGDIOBJ, SRCCOPY,
};
use windows_sys::Win32::Media::{timeBeginPeriod, timeEndPeriod};
use windows_sys::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClientRect, GetSystemMetrics, GetWindowTextW, IsIconic, IsWindow, IsWindowVisible, SM_CXSCREEN,
    SM_CYSCREEN,
};

/// Fenêtre de WoW (titre « World of Warcraft... », visible), ou nulle. Parmi plusieurs fenêtres de
/// ce titre (page web « World of Warcraft... » dans un navigateur), celle dont l'exécutable est
/// dans un dossier de version du jeu (`_retail_`, `_forever_`...) l'emporte.
pub fn wow_window() -> HWND {
    unsafe extern "system" fn cb(h: HWND, lp: LPARAM) -> windows_sys::core::BOOL {
        if IsWindowVisible(h) != 0 && title_of(h).starts_with("World of Warcraft") {
            (*(lp as *mut Vec<HWND>)).push(h);
        }
        1
    }
    let mut found: Vec<HWND> = Vec::new();
    unsafe {
        EnumWindows(Some(cb), &mut found as *mut Vec<HWND> as LPARAM);
    }
    match found.len() {
        0 => null_mut(),
        1 => found[0],
        _ => found.iter().copied().find(|h| game_of(*h).is_some()).unwrap_or(found[0]),
    }
}

/// Toutes les fenêtres de WoW ouvertes (0.15 : plusieurs jeux à la fois, multi-comptes ou
/// plusieurs versions), dans l'ordre d'empilement de Windows ; seulement celles dont l'exécutable
/// est dans un dossier de version du jeu.
pub fn wow_windows() -> Vec<HWND> {
    unsafe extern "system" fn cb(h: HWND, lp: LPARAM) -> windows_sys::core::BOOL {
        if IsWindowVisible(h) != 0 && title_of(h).starts_with("World of Warcraft") {
            (*(lp as *mut Vec<HWND>)).push(h);
        }
        1
    }
    let mut found: Vec<HWND> = Vec::new();
    unsafe {
        EnumWindows(Some(cb), &mut found as *mut Vec<HWND> as LPARAM);
    }
    found.retain(|h| game_of(*h).is_some());
    found
}

/// Titre d'une fenêtre d'un autre programme ; vide pour les fenêtres de l'application elle-même.
/// GetWindowTextW envoie un message (WM_GETTEXT) aux fenêtres de son propre programme et attend
/// que leur fil le traite : appelé depuis le fil de capture pendant que le fil principal attendait
/// ce même fil, l'application restait bloquée avant le début de la traduction (0.13.3 et 0.14.0 :
/// présence plus écrite, rien de redessiné). Pour les fenêtres des autres programmes, Windows rend
/// le titre sans message.
// Poignée de fenêtre transmise à Windows, jamais lue par le programme.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn title_of(h: HWND) -> String {
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, &mut pid);
        if pid == 0 || pid == GetCurrentProcessId() {
            return String::new();
        }
        let mut buf = [0u16; 64];
        let n = GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32);
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }
}

/// Version du jeu ouverte (« _retail_ », « _forever_ »...) : dossier de son exécutable.
pub fn wow_flavor() -> Option<String> {
    wow_game().map(|(_, flavor)| flavor)
}

/// Jeu ouvert : (dossier World of Warcraft, version « _retail_ », « _forever_ »...), d'après le
/// chemin de son exécutable.
pub fn wow_game() -> Option<(std::path::PathBuf, String)> {
    let h = wow_window();
    if h.is_null() {
        return None;
    }
    game_of(h)
}

/// (dossier World of Warcraft, version) de l'exécutable d'une fenêtre, s'il est dans un dossier de
/// version du jeu.
// Poignée de fenêtre transmise à Windows, jamais lue par le programme.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn game_of(h: HWND) -> Option<(std::path::PathBuf, String)> {
    let dir = exe_of(h)?.parent()?.to_path_buf();
    let flavor = dir.file_name()?.to_string_lossy().into_owned();
    if !(flavor.len() > 2 && flavor.starts_with('_') && flavor.ends_with('_')) {
        return None;
    }
    Some((dir.parent()?.to_path_buf(), flavor))
}

/// Exécutable du processus d'une fenêtre.
fn exe_of(h: HWND) -> Option<std::path::PathBuf> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, &mut pid);
        if pid == 0 {
            return None;
        }
        let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if p.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut n);
        CloseHandle(p);
        if ok == 0 {
            return None;
        }
        Some(std::path::PathBuf::from(String::from_utf16_lossy(&buf[..n as usize])))
    }
}

/// Zone de jeu d'une fenêtre à l'écran : (gauche, haut, droite, bas), pixels physiques.
// Poignée de fenêtre transmise à Windows, jamais lue par le programme.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn client_rect(h: HWND) -> Option<(i32, i32, i32, i32)> {
    unsafe {
        if h.is_null() || IsWindow(h) == 0 || IsIconic(h) != 0 {
            return None;
        }
        let mut r: RECT = std::mem::zeroed();
        if GetClientRect(h, &mut r) == 0 || r.right <= 0 || r.bottom <= 0 {
            return None;
        }
        let mut tl = POINT { x: 0, y: 0 };
        let mut br = POINT { x: r.right, y: r.bottom };
        ClientToScreen(h, &mut tl);
        ClientToScreen(h, &mut br);
        Some((tl.x, tl.y, br.x, br.y))
    }
}

/// Zone de jeu de WoW à l'écran, s'il est ouvert et pas réduit.
pub fn wow_client_rect() -> Option<(i32, i32, i32, i32)> {
    client_rect(wow_window())
}

/// Écran où se trouve WoW (0.13.3, télévision 4K) : nom, télévision, liaison, HDR, définition,
/// échelle de Windows et zone de jeu, pour le journal ; vrai si c'est une télévision (0.14).
pub fn wow_screen_info() -> Option<(String, bool)> {
    screen_info_of(wow_window())
}

/// Comme `wow_screen_info`, pour une fenêtre de WoW donnée (0.15, plusieurs jeux).
// Poignée de fenêtre transmise à Windows, jamais lue par le programme.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn screen_info_of(h: HWND) -> Option<(String, bool)> {
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    if h.is_null() {
        return None;
    }
    let (l, t, r, b) = client_rect(h)?;
    let s = screen_of(h)?;
    let dpi = unsafe { GetDpiForWindow(h) };
    Some((
        format!("WoW sur {} ; échelle {} %, zone de jeu {}x{} en ({l}, {t})", s.describe(), dpi * 100 / 96, r - l, b - t),
        s.is_tv(),
    ))
}

/// Écrans branchés (0.14) : nom (EDID), liaison, HDR, taille, définition. Vide si Windows ne
/// répond pas.
pub fn screens() -> Vec<crate::ecran::Screen> {
    use crate::ecran::{Hdr, Screen};
    use windows_sys::Win32::Devices::Display::{
        DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig, DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
        DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO,
        DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME,
        QDC_ONLY_ACTIVE_PATHS,
    };
    use windows_sys::Win32::Graphics::Gdi::{EnumDisplaySettingsW, DEVMODEW, ENUM_CURRENT_SETTINGS};
    let text = |w: &[u16]| {
        let n = w.iter().position(|c| *c == 0).unwrap_or(w.len());
        String::from_utf16_lossy(&w[..n])
    };
    let mut out = Vec::new();
    unsafe {
        let (mut np, mut nm) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm) != 0 || np == 0 {
            return out;
        }
        let mut paths: Vec<DISPLAYCONFIG_PATH_INFO> = vec![std::mem::zeroed(); np as usize];
        let mut modes: Vec<DISPLAYCONFIG_MODE_INFO> = vec![std::mem::zeroed(); nm.max(1) as usize];
        if QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), null_mut()) != 0 {
            return out;
        }
        for p in &paths[..np as usize] {
            let mut src: DISPLAYCONFIG_SOURCE_DEVICE_NAME = std::mem::zeroed();
            src.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            src.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
            src.header.adapterId = p.sourceInfo.adapterId;
            src.header.id = p.sourceInfo.id;
            if DisplayConfigGetDeviceInfo(&mut src.header) != 0 {
                continue;
            }
            let mut s = Screen { gdi: text(&src.viewGdiDeviceName), ..Default::default() };
            let mut tgt: DISPLAYCONFIG_TARGET_DEVICE_NAME = std::mem::zeroed();
            tgt.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
            tgt.header.size = std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
            tgt.header.adapterId = p.targetInfo.adapterId;
            tgt.header.id = p.targetInfo.id;
            if DisplayConfigGetDeviceInfo(&mut tgt.header) == 0 {
                s.name = text(&tgt.monitorFriendlyDeviceName);
                s.link = tgt.outputTechnology as u32;
                // Taille (et nom, s'il manque) dans l'EDID gardé par Windows.
                if let Some(key) = crate::ecran::edid_key(&text(&tgt.monitorDevicePath)) {
                    if let Some((size, name)) = edid_of(&key).as_deref().and_then(crate::ecran::parse_edid) {
                        s.size_cm = size;
                        if s.name.is_empty() {
                            s.name = name.unwrap_or_default();
                        }
                    }
                }
            } else {
                s.link = p.targetInfo.outputTechnology as u32;
            }
            let mut color: DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO = std::mem::zeroed();
            color.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO;
            color.header.size = std::mem::size_of::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>() as u32;
            color.header.adapterId = p.targetInfo.adapterId;
            color.header.id = p.targetInfo.id;
            if DisplayConfigGetDeviceInfo(&mut color.header) == 0 {
                let v = color.Anonymous.value;
                s.hdr = match (v & 1 != 0, v & 2 != 0) {
                    (_, true) => Hdr::On,
                    (true, false) => Hdr::Off,
                    _ => Hdr::No,
                };
            }
            let mut dm: DEVMODEW = std::mem::zeroed();
            dm.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
            let gdi: Vec<u16> = s.gdi.encode_utf16().chain(Some(0)).collect();
            if EnumDisplaySettingsW(gdi.as_ptr(), ENUM_CURRENT_SETTINGS, &mut dm) != 0 {
                s.w = dm.dmPelsWidth;
                s.h = dm.dmPelsHeight;
                s.hz = dm.dmDisplayFrequency;
                let pos = dm.Anonymous1.Anonymous2.dmPosition;
                s.primary = pos.x == 0 && pos.y == 0;
            }
            if !out.iter().any(|o: &Screen| o.gdi == s.gdi) {
                out.push(s);
            }
        }
    }
    out
}

/// EDID d'un écran, lu dans le registre.
fn edid_of(key: &str) -> Option<Vec<u8>> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_BINARY};
    let k: Vec<u16> = key.encode_utf16().chain(Some(0)).collect();
    let v: Vec<u16> = "EDID".encode_utf16().chain(Some(0)).collect();
    let mut buf = vec![0u8; 1024];
    let mut len = buf.len() as u32;
    unsafe {
        if RegGetValueW(HKEY_LOCAL_MACHINE, k.as_ptr(), v.as_ptr(), RRF_RT_REG_BINARY, null_mut(), buf.as_mut_ptr() as *mut c_void, &mut len) != 0 {
            return None;
        }
    }
    buf.truncate(len as usize);
    Some(buf)
}

/// Écran où se trouve WoW (périphérique de Windows « \\.\DISPLAYn »).
pub fn wow_monitor_device() -> Option<String> {
    monitor_device_of(wow_window())
}

// Poignée de fenêtre transmise à Windows, jamais lue par le programme.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn monitor_device_of(h: HWND) -> Option<String> {
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST};
    if h.is_null() {
        return None;
    }
    unsafe {
        let mon = MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST);
        let mut mi: MONITORINFOEXW = std::mem::zeroed();
        mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(mon, &mut mi as *mut MONITORINFOEXW as *mut _) == 0 {
            return None;
        }
        let n = mi.szDevice.iter().position(|c| *c == 0).unwrap_or(mi.szDevice.len());
        Some(String::from_utf16_lossy(&mi.szDevice[..n]))
    }
}

/// Écran de WoW, décrit (nom, télévision, HDR...), s'il est ouvert.
pub fn wow_screen() -> Option<crate::ecran::Screen> {
    screen_of(wow_window())
}

/// Écran d'une fenêtre, décrit.
// Poignée de fenêtre transmise à Windows, jamais lue par le programme.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn screen_of(h: HWND) -> Option<crate::ecran::Screen> {
    let dev = monitor_device_of(h)?;
    let found = screens().into_iter().find(|s| s.gdi.eq_ignore_ascii_case(&dev));
    Some(found.unwrap_or(crate::ecran::Screen { gdi: dev, ..Default::default() }))
}

pub struct Capturer {
    screen_dc: HDC,
    mem_dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    pub w: usize,
    pub h: usize,
    x: i32,
    y: i32,
    pub screen_w: i32,
    pub screen_h: i32,
    /// Fenêtre de WoW suivie (recherchée de nouveau chaque seconde).
    wow: HWND,
    looked: Option<std::time::Instant>,
    /// Largeur demandée (0 : selon la largeur de WoW).
    asked_w: usize,
    /// Fenêtre imposée (0.15, plusieurs jeux) : jamais remplacée par une autre fenêtre de WoW.
    fixed: bool,
    /// Hauteur de la zone large (avant resserrement sur le bandeau).
    full_h: usize,
    /// Zone resserrée sur le bandeau trouvé (0.15) : (largeur, hauteur) ; `None` : zone large.
    narrow: Option<(usize, usize)>,
}

impl Capturer {
    /// `width` à 0 : 30 % de la largeur de l'écran, de quoi contenir le bandeau (25 %).
    pub fn new(width: usize, height: usize) -> Result<Self, String> {
        Self::new_for(width, height, null_mut())
    }

    /// Capture du bandeau d'une fenêtre de WoW donnée (nulle : la fenêtre de WoW trouvée, suivie).
    // Poignée de fenêtre transmise à Windows, jamais lue par le programme.
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn new_for(width: usize, height: usize, target: HWND) -> Result<Self, String> {
        let fixed = !target.is_null();
        let start = if fixed { target } else { wow_window() };
        unsafe {
            // Coordonnées en pixels physiques, quelle que soit la mise à l'échelle Windows.
            SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            timeBeginPeriod(1);
            let sw = GetSystemMetrics(SM_CXSCREEN);
            let sh = GetSystemMetrics(SM_CYSCREEN);
            if sw <= 0 || sh <= 0 {
                return Err("taille d'écran introuvable".into());
            }
            // Largeur de WoW s'il est ouvert (télévision 4K à côté d'un écran plus petit, 0.13.3),
            // sinon celle de l'écran principal ; suivie ensuite (voir follow_wow).
            let base = client_rect(start).map_or(sw, |(l, _, r, _)| (r - l).max(sw.min(640)));
            let w = if width == 0 {
                (base as f32 * 0.30) as i32 + 8
            } else {
                width as i32
            }
            .min(base.max(sw));
            let h = (height as i32).min(sh);

            let screen_dc = GetDC(null_mut());
            if screen_dc.is_null() {
                return Err("GetDC a échoué".into());
            }
            let mem_dc = CreateCompatibleDC(screen_dc);
            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = w;
            bmi.bmiHeader.biHeight = -h; // lignes de haut en bas
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB;
            let mut bits: *mut c_void = null_mut();
            let bmp = CreateDIBSection(screen_dc, &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if mem_dc.is_null() || bmp.is_null() || bits.is_null() {
                ReleaseDC(null_mut(), screen_dc);
                return Err("CreateDIBSection a échoué".into());
            }
            let old = SelectObject(mem_dc, bmp);
            Ok(Capturer {
                screen_dc,
                mem_dc,
                bmp,
                old,
                bits: bits as *mut u8,
                w: w as usize,
                h: h as usize,
                x: sw - w,
                y: sh - h,
                screen_w: sw,
                screen_h: sh,
                wow: if fixed { target } else { null_mut() },
                looked: None,
                asked_w: width,
                fixed,
                full_h: h as usize,
                narrow: None,
            })
        }
    }

    /// Place la zone de capture au coin bas-droit de la zone de jeu de WoW.
    fn follow_wow(&mut self) {
        let stale = self.looked.map_or(true, |t| t.elapsed() > std::time::Duration::from_secs(1));
        if !self.fixed && (stale || self.wow.is_null()) {
            self.wow = wow_window();
            self.looked = Some(std::time::Instant::now());
        }
        if let Some((l, _, r, b)) = client_rect(self.wow) {
            // Zone resserrée sur le bandeau (0.15) : sa taille ne suit plus celle de WoW.
            if let Some((w, h)) = self.narrow {
                if (w, h) != (self.w, self.h) {
                    self.resize(w, h);
                }
            } else if self.asked_w == 0 {
                // Zone de capture à la taille du bandeau de ce WoW (30 % de sa largeur).
                let want = (((r - l) as f32 * 0.30) as i32 + 8).max(64) as usize;
                if want.abs_diff(self.w) > 16 || self.h != self.full_h {
                    self.resize(want, self.full_h);
                }
            } else if self.h != self.full_h || self.w != self.asked_w {
                self.resize(self.asked_w, self.full_h);
            }
            self.x = r - self.w as i32;
            self.y = b - self.h as i32;
        }
    }

    /// La fenêtre de WoW capturée existe-t-elle encore ?
    pub fn target_alive(&self) -> bool {
        unsafe { !self.wow.is_null() && IsWindow(self.wow) != 0 }
    }

    /// Coin haut-gauche de la zone capturée (pixels du bureau).
    pub fn origin(&self) -> (i32, i32) {
        (self.x, self.y)
    }

    /// Bandeau trouvé (0.15) : la zone capturée se resserre sur lui (sa largeur et deux blocs de
    /// marge, plus quelques rangées au-dessus pour le cache du bandeau). Environ 4 fois moins de
    /// pixels lus à chaque capture.
    pub fn narrow_to(&mut self, g: &crate::decode::Geometry) {
        // Bandeau collé au coin bas-droit de la zone capturée seulement (sinon, rien à resserrer).
        if g.right + 4 < self.w || g.bottom + 4 < self.h || g.block == 0 || g.cols == 0 {
            return;
        }
        self.narrow = Some(crate::bandeau::narrow_size(g));
    }

    /// Bandeau perdu : retour à la zone large.
    pub fn widen(&mut self) {
        self.narrow = None;
    }

    pub fn narrowed(&self) -> bool {
        self.narrow.is_some()
    }

    /// Nouvelle taille de la zone de capture (WoW redimensionné, autre écran, zone resserrée).
    fn resize(&mut self, w: usize, h: usize) {
        unsafe {
            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = w as i32;
            bmi.bmiHeader.biHeight = -(h as i32);
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB;
            let mut bits: *mut c_void = null_mut();
            let bmp = CreateDIBSection(self.screen_dc, &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if bmp.is_null() || bits.is_null() {
                return;
            }
            SelectObject(self.mem_dc, bmp);
            DeleteObject(self.bmp);
            self.bmp = bmp;
            self.bits = bits as *mut u8;
            self.w = w;
            self.h = h;
        }
    }

    /// Copie la zone et renvoie les pixels BGRA (lignes de haut en bas).
    pub fn grab(&mut self) -> Result<&[u8], String> {
        self.follow_wow();
        unsafe {
            if BitBlt(
                self.mem_dc,
                0,
                0,
                self.w as i32,
                self.h as i32,
                self.screen_dc,
                self.x,
                self.y,
                SRCCOPY,
            ) == 0
            {
                return Err("BitBlt a échoué (écran verrouillé ou session changée ?)".into());
            }
            GdiFlush();
            Ok(std::slice::from_raw_parts(self.bits, self.w * self.h * 4))
        }
    }
}

impl Drop for Capturer {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.mem_dc, self.old);
            DeleteObject(self.bmp);
            DeleteDC(self.mem_dc);
            ReleaseDC(null_mut(), self.screen_dc);
            timeEndPeriod(1);
        }
    }
}
