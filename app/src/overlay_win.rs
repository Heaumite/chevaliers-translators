//! Fenêtres du chat miroir (Windows) : une fenêtre transparente par fenêtre de chat, toujours au
//! premier plan (WoW en « fenêtré plein écran » l'est souvent aussi : un miroir qui ne l'était pas
//! restait dessous, essai du 09/11/2026 en 0.12.0), traversée par la souris, absente de la barre des
//! tâches, jamais active. Une autre application passée devant WoW : rien n'est dessiné sous elle
//! (rectangle de la fenêtre active, `window_over_wow`). Son contenu est une image BGRA prémultipliée (UpdateLayeredWindow).
//! WoW doit être en mode « Fenêtré (plein écran) » : le plein écran exclusif masque
//! toute fenêtre posée au-dessus.
//!
//! Icônes de copie (0.11) : une seconde fenêtre par fenêtre de chat, réduite au rectangle des
//! icônes, presque invisible (opacité 1/255 sur les icônes, 0 ailleurs). Windows laisse passer la
//! souris là où l'opacité est nulle : seul un clic sur une icône lui arrive. Elle ne prend jamais
//! le focus (WoW reste la fenêtre active) et montre la main au survol.

use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::Mutex;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
    HBITMAP, HDC, HGDIOBJ,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EnumWindows, GetForegroundWindow, IsIconic,
    IsWindowVisible, LoadCursorW, SetCursor, SetForegroundWindow, SW_RESTORE,
    GetSystemMetrics, PeekMessageW, RegisterClassExW, ShowWindow, TranslateMessage, UpdateLayeredWindow, MSG,
    IDC_HAND, MA_NOACTIVATE, PM_REMOVE, SM_CYSCREEN, SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA, WM_LBUTTONDBLCLK,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR, WNDCLASSEXW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP, SetWindowPos,
    GetWindow, GetWindowRect, GW_HWNDNEXT, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE,
};

use crate::mirror::{Canvas, Hit};

type WndProc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

unsafe extern "system" fn wndproc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    DefWindowProcW(h, m, w, l)
}

/// Clics reçus par les fenêtres des icônes : (fenêtre, x, y dans la fenêtre).
static CLICKS: Mutex<Vec<(isize, i32, i32)>> = Mutex::new(Vec::new());

unsafe extern "system" fn click_proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match m {
        // Un clic ne rend jamais cette fenêtre active : WoW garde le clavier.
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_SETCURSOR => {
            SetCursor(LoadCursorW(null_mut(), IDC_HAND));
            1
        }
        WM_LBUTTONDOWN => {
            let x = (l & 0xFFFF) as u16 as i16 as i32;
            let y = ((l >> 16) & 0xFFFF) as u16 as i16 as i32;
            if let Ok(mut c) = CLICKS.lock() {
                c.push((h as isize, x, y));
            }
            0
        }
        WM_LBUTTONUP | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN | WM_RBUTTONUP => 0,
        _ => DefWindowProcW(h, m, w, l),
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

struct Win {
    hwnd: HWND,
    mem_dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    w: usize,
    h: usize,
    shown: bool,
    /// Fenêtre des icônes : coin haut-gauche dans l'image de la fenêtre de chat.
    off: (i32, i32),
}

impl Win {
    fn new(hwnd: HWND) -> Win {
        Win { hwnd, mem_dc: null_mut(), bmp: null_mut(), old: null_mut(), bits: null_mut(), w: 0, h: 0, shown: false, off: (0, 0) }
    }

    fn free_surface(&mut self) {
        unsafe {
            if !self.mem_dc.is_null() {
                SelectObject(self.mem_dc, self.old);
                DeleteObject(self.bmp);
                DeleteDC(self.mem_dc);
            }
        }
        self.mem_dc = null_mut();
        self.bits = null_mut();
    }

    fn ensure_surface(&mut self, w: usize, h: usize) -> Result<(), String> {
        if !self.mem_dc.is_null() && self.w == w && self.h == h {
            return Ok(());
        }
        self.free_surface();
        unsafe {
            let screen = GetDC(null_mut());
            let mem_dc = CreateCompatibleDC(screen);
            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = w as i32;
            bmi.bmiHeader.biHeight = -(h as i32);
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB;
            let mut bits: *mut c_void = null_mut();
            let bmp = CreateDIBSection(screen, &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            ReleaseDC(null_mut(), screen);
            if mem_dc.is_null() || bmp.is_null() || bits.is_null() {
                return Err("CreateDIBSection a échoué (miroir)".into());
            }
            self.old = SelectObject(mem_dc, bmp);
            self.mem_dc = mem_dc;
            self.bmp = bmp;
            self.bits = bits as *mut u8;
            self.w = w;
            self.h = h;
        }
        Ok(())
    }

    /// Pose l'image `px` (w x h) avec son coin haut-gauche en (`x`, `y`), pixels de l'écran.
    fn present(&mut self, x: i32, y: i32, w: usize, h: usize, px: &[u32]) -> Result<(), String> {
        self.ensure_surface(w, h)?;
        unsafe {
            let dst = std::slice::from_raw_parts_mut(self.bits as *mut u32, w * h);
            dst.copy_from_slice(px);
            let screen = GetDC(null_mut());
            let pt = POINT { x, y };
            let size = SIZE { cx: w as i32, cy: h as i32 };
            let src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let ok = UpdateLayeredWindow(self.hwnd, screen, &pt, &size, self.mem_dc, &src, 0, &blend, ULW_ALPHA);
            ReleaseDC(null_mut(), screen);
            if ok == 0 {
                return Err("UpdateLayeredWindow a échoué (miroir)".into());
            }
            if !self.shown {
                ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                self.shown = true;
                self.place();
            }
        }
        Ok(())
    }

    /// Repasse en tête des fenêtres « toujours au premier plan » (un WoW qui l'est aussi passe
    /// devant quand il redevient actif).
    fn place(&self) {
        unsafe {
            SetWindowPos(self.hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
        }
    }

    fn hide(&mut self) {
        if self.shown {
            unsafe {
                ShowWindow(self.hwnd, SW_HIDE);
            }
            self.shown = false;
        }
    }
}

pub struct Overlay {
    wins: HashMap<u8, Win>,
    clicks: HashMap<u8, Win>,
    class: Vec<u16>,
    click_class: Vec<u16>,
    screen_h: i32,
    /// Coin bas-gauche de la zone de jeu de WoW à l'écran (pixels du bureau) : origine des
    /// positions du miroir. WoW sur un autre écran que le principal (télévision 4K, 0.13.3) ou en
    /// fenêtre : le miroir le suit au lieu de se dessiner sur l'écran principal.
    origin: (i32, i32),
    /// Fenêtres exclues des captures d'écran (cache du bandeau).
    covers: Vec<u8>,
}

/// Windows 10 version 2004 (build 19041) ou suivant : WDA_EXCLUDEFROMCAPTURE existe (avant, la
/// fenêtre apparaîtrait noire dans la capture, et le bandeau serait illisible).
pub fn capture_exclusion_supported() -> bool {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
    let key: Vec<u16> = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion".encode_utf16().chain(Some(0)).collect();
    let name: Vec<u16> = "CurrentBuildNumber".encode_utf16().chain(Some(0)).collect();
    let mut buf = [0u16; 32];
    let mut len = (buf.len() * 2) as u32;
    let ok = unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, null_mut(), buf.as_mut_ptr() as *mut c_void, &mut len)
    } == 0;
    if !ok {
        return false;
    }
    let n = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n]).trim().parse::<u32>().is_ok_and(|b| b >= 19041)
}

impl Overlay {
    /// À créer après le capteur d'écran (qui règle la prise en compte de l'échelle Windows).
    pub fn new() -> Result<Overlay, String> {
        let class = wide("CT_ChatMiroir");
        let click_class = wide("CT_ChatMiroirCopie");
        unsafe {
            for (name, proc_) in [(&class, wndproc as WndProc), (&click_class, click_proc as WndProc)] {
                let wc = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    style: 0,
                    lpfnWndProc: Some(proc_),
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hInstance: GetModuleHandleW(std::ptr::null()),
                    hIcon: null_mut(),
                    hCursor: null_mut(),
                    hbrBackground: null_mut(),
                    lpszMenuName: std::ptr::null(),
                    lpszClassName: name.as_ptr(),
                    hIconSm: null_mut(),
                };
                // Déjà enregistrée : traduction relancée (WoW lancé dans une autre version).
                if RegisterClassExW(&wc) == 0 && windows_sys::Win32::Foundation::GetLastError() != 1410 {
                    return Err("RegisterClassExW a échoué (miroir)".into());
                }
            }
            Ok(Overlay {
                wins: HashMap::new(),
                clicks: HashMap::new(),
                class,
                click_class,
                screen_h: GetSystemMetrics(SM_CYSCREEN),
                origin: (0, GetSystemMetrics(SM_CYSCREEN)),
                covers: Vec::new(),
            })
        }
    }

    fn create(class: &[u16], title: &str, ex_style: u32) -> Result<HWND, String> {
        let title = wide(title);
        let hwnd = unsafe {
            CreateWindowExW(
                ex_style,
                class.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                null_mut(),
                null_mut(),
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return Err("CreateWindowExW a échoué (miroir)".into());
        }
        Ok(hwnd)
    }

    fn window(&mut self, frame: u8) -> Result<&mut Win, String> {
        if !self.wins.contains_key(&frame) {
            let hwnd = Self::create(
                &self.class,
                &format!("CT miroir {frame}"),
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            )?;
            self.wins.insert(frame, Win::new(hwnd));
        }
        Ok(self.wins.get_mut(&frame).unwrap())
    }

    fn click_window(&mut self, frame: u8) -> Result<&mut Win, String> {
        if !self.clicks.contains_key(&frame) {
            // Pas de WS_EX_TRANSPARENT : les pixels d'opacité non nulle reçoivent la souris.
            let hwnd = Self::create(
                &self.click_class,
                &format!("CT copie {frame}"),
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            )?;
            self.clicks.insert(frame, Win::new(hwnd));
        }
        Ok(self.clicks.get_mut(&frame).unwrap())
    }

    /// Affiche l'image d'une fenêtre de chat ; `left`, `bottom` en pixels depuis le bas à gauche.
    /// `hits` : icônes de copie cliquables (pixels de l'image).
    pub fn show(&mut self, frame: u8, left: i32, bottom: i32, cv: &Canvas, hits: &[Hit]) -> Result<(), String> {
        let top = self.origin.1 - bottom - cv.h as i32;
        let left = self.origin.0 + left;
        self.window(frame)?.present(left, top, cv.w, cv.h, &cv.px)?;
        // Fenêtre des icônes, réduite à leur rectangle commun.
        let (cw, ch) = (cv.w as i32, cv.h as i32);
        let x0 = hits.iter().map(|h| h.x).min().unwrap_or(0).clamp(0, cw);
        let y0 = hits.iter().map(|h| h.y).min().unwrap_or(0).clamp(0, ch);
        let x1 = hits.iter().map(|h| h.x + h.w).max().unwrap_or(0).clamp(0, cw);
        let y1 = hits.iter().map(|h| h.y + h.h).max().unwrap_or(0).clamp(0, ch);
        if hits.is_empty() || x1 <= x0 || y1 <= y0 {
            if let Some(c) = self.clicks.get_mut(&frame) {
                c.hide();
            }
            return Ok(());
        }
        let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
        let mut px = vec![0u32; w * h];
        for t in hits {
            for y in t.y.max(y0)..(t.y + t.h).min(y1) {
                for x in t.x.max(x0)..(t.x + t.w).min(x1) {
                    // Opacité 1/255, noir prémultiplié : invisible, mais cliquable.
                    px[(y - y0) as usize * w + (x - x0) as usize] = 0x0100_0000;
                }
            }
        }
        let c = self.click_window(frame)?;
        c.off = (x0, y0);
        c.present(left + x0, top + y0, w, h, &px)
    }

    /// Zone de jeu de WoW (gauche, haut, droite, bas en pixels du bureau) : origine du miroir.
    pub fn set_wow_rect(&mut self, r: Option<(i32, i32, i32, i32)>) {
        if let Some((l, _, _, b)) = r {
            self.origin = (l, b);
        }
    }

    pub fn origin(&self) -> (i32, i32) {
        self.origin
    }

    /// Panneau placé en pixels du bureau (coin haut-gauche), sans icône cliquable.
    pub fn show_screen(&mut self, frame: u8, left: i32, top: i32, cv: &Canvas) -> Result<(), String> {
        self.window(frame)?.present(left, top, cv.w, cv.h, &cv.px)?;
        if let Some(c) = self.clicks.get_mut(&frame) {
            c.hide();
        }
        Ok(())
    }

    /// Cache du bandeau (0.15) : image posée sur le bandeau, vue par le joueur mais absente des
    /// captures d'écran (WDA_EXCLUDEFROMCAPTURE, Windows 10 2004 et suivants) : la capture lit
    /// toujours le bandeau dessous. Erreur si Windows ne l'accepte pas (rien n'est alors affiché :
    /// le cache rendrait le bandeau illisible).
    pub fn show_cover(&mut self, frame: u8, left: i32, top: i32, cv: &Canvas) -> Result<(), String> {
        use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE};
        if !self.covers.contains(&frame) {
            if !capture_exclusion_supported() {
                return Err("Windows trop ancien pour cacher le bandeau (10 version 2004 au moins)".into());
            }
            let w = self.window(frame)?;
            if unsafe { SetWindowDisplayAffinity(w.hwnd, WDA_EXCLUDEFROMCAPTURE) } == 0 {
                return Err(format!("SetWindowDisplayAffinity refusé ({})", unsafe { windows_sys::Win32::Foundation::GetLastError() }));
            }
            self.covers.push(frame);
        }
        self.window(frame)?.present(left, top, cv.w, cv.h, &cv.px)
    }

    pub fn hide(&mut self, frame: u8) {
        if let Some(w) = self.wins.get_mut(&frame) {
            w.hide();
        }
        if let Some(w) = self.clicks.get_mut(&frame) {
            w.hide();
        }
    }

    /// Fenêtre active passée devant WoW (une autre application) : son rectangle visible (gauche,
    /// haut, droite, bas), pixels de l'écran. Rien si WoW est actif, ou si la fenêtre active est
    /// derrière WoW (WoW « toujours au premier plan »), réduite ou masquée.
    pub fn window_over_wow() -> Option<(i32, i32, i32, i32)> {
        Self::window_over(crate::capture_win::wow_window())
    }

    /// Comme `window_over_wow`, pour une fenêtre de WoW donnée (0.15) : un autre WoW passé devant
    /// compte comme une autre application.
    // Poignée de fenêtre transmise à Windows, jamais lue par le programme.
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn window_over(wow: HWND) -> Option<(i32, i32, i32, i32)> {
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
        unsafe {
            let fg = GetForegroundWindow();
            if wow.is_null() || fg.is_null() || fg == wow || IsIconic(fg) != 0 || IsWindowVisible(fg) == 0 {
                return None;
            }
            // Devant WoW ? On descend l'ordre d'empilement depuis la fenêtre active.
            let mut h = fg;
            let mut above = false;
            for _ in 0..2000 {
                h = GetWindow(h, GW_HWNDNEXT);
                if h.is_null() {
                    break;
                }
                if h == wow {
                    above = true;
                    break;
                }
            }
            if !above {
                return None;
            }
            let mut cloaked = 0u32;
            let ok = DwmGetWindowAttribute(fg, DWMWA_CLOAKED as u32, &mut cloaked as *mut u32 as *mut c_void, 4);
            if ok == 0 && cloaked != 0 {
                return None;
            }
            let mut r: RECT = std::mem::zeroed();
            let exact = DwmGetWindowAttribute(
                fg,
                DWMWA_EXTENDED_FRAME_BOUNDS as u32,
                &mut r as *mut RECT as *mut c_void,
                std::mem::size_of::<RECT>() as u32,
            ) == 0;
            if !exact && GetWindowRect(fg, &mut r) == 0 {
                return None;
            }
            (r.right > r.left && r.bottom > r.top).then_some((r.left, r.top, r.right, r.bottom))
        }
    }

    /// Hauteur de l'écran principal (pixels).
    pub fn screen_height(&self) -> i32 {
        self.screen_h
    }

    /// WoW actif : les fenêtres affichées repassent devant lui (un clic dans WoW le met devant).
    pub fn raise_all(&self) {
        for w in self.wins.values().chain(self.clicks.values()) {
            if w.shown {
                w.place();
            }
        }
    }

    pub fn hide_all(&mut self) {
        for w in self.wins.values_mut().chain(self.clicks.values_mut()) {
            w.hide();
        }
    }

    /// Clics sur les icônes de copie depuis le dernier appel : (fenêtre de chat, x, y dans son
    /// image).
    pub fn take_clicks(&mut self) -> Vec<(u8, i32, i32)> {
        // Seulement les clics de ses propres fenêtres : ceux d'un autre jeu (0.15) restent pour lui.
        let mine: Vec<isize> = self.clicks.values().map(|w| w.hwnd as isize).collect();
        let raw: Vec<(isize, i32, i32)> = match CLICKS.lock() {
            Ok(mut c) => {
                let (take, keep): (Vec<_>, Vec<_>) = c.drain(..).partition(|(h, _, _)| mine.contains(h));
                *c = keep;
                take
            }
            Err(_) => Vec::new(),
        };
        raw.into_iter()
            .filter_map(|(h, x, y)| {
                self.clicks
                    .iter()
                    .find(|(_, w)| w.hwnd as isize == h && w.shown)
                    .map(|(f, w)| (*f, x + w.off.0, y + w.off.1))
            })
            .collect()
    }

    /// Vrai si la fenêtre active est cette fenêtre de WoW (0.15, plusieurs jeux).
    pub fn in_front_of(h: HWND) -> bool {
        unsafe { !h.is_null() && GetForegroundWindow() == h }
    }

    /// Vrai si la fenêtre active est celle de WoW (titre « World of Warcraft »).
    pub fn wow_in_front() -> bool {
        unsafe {
            let fg = GetForegroundWindow();
            if fg.is_null() {
                return false;
            }
            crate::capture_win::title_of(fg).starts_with("World of Warcraft")
        }
    }

    /// Fenêtre de WoW (titre « World of Warcraft... », visible).
    fn find_wow() -> HWND {
        unsafe extern "system" fn cb(h: HWND, lp: LPARAM) -> windows_sys::core::BOOL {
            if IsWindowVisible(h) != 0 && crate::capture_win::title_of(h).starts_with("World of Warcraft") {
                *(lp as *mut HWND) = h;
                return 0;
            }
            1
        }
        let mut found: HWND = null_mut();
        unsafe {
            EnumWindows(Some(cb), &mut found as *mut HWND as LPARAM);
        }
        found
    }

    /// Met WoW au premier plan (permis à un programme lancé depuis la fenêtre active).
    pub fn bring_wow_to_front() -> bool {
        let h = Self::find_wow();
        if h.is_null() {
            return false;
        }
        unsafe {
            if IsIconic(h) != 0 {
                ShowWindow(h, SW_RESTORE);
            }
            SetForegroundWindow(h) != 0
        }
    }

    /// Ramène la console devant à la fin d'un test (sinon Windows fait clignoter son bouton).
    pub fn bring_console_to_front() {
        unsafe {
            let h = windows_sys::Win32::System::Console::GetConsoleWindow();
            if !h.is_null() {
                SetForegroundWindow(h);
            }
        }
    }

    /// Traite les messages Windows des fenêtres du miroir (à appeler régulièrement).
    pub fn pump(&self) {
        Self::pump_thread();
    }

    /// Messages de toutes les fenêtres du fil (miroirs de tous les jeux, icône de l'application).
    pub fn pump_thread() {
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        for (_, mut w) in self.wins.drain().chain(self.clicks.drain()) {
            w.free_surface();
            unsafe {
                DestroyWindow(w.hwnd);
            }
        }
    }
}
