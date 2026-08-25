#[macro_use]
mod logging;
mod harness;
mod theme;

use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::Duration;

use tauri::image::Image;
use tauri::tray::TrayIcon;
use tauri::{
    AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

use harness::HarnessManager;
use theme::Effective;

static HARNESS: Mutex<Option<Arc<HarnessManager>>> = Mutex::new(None);

/// Tray icon handle kept around so the icon can follow theme switches.
static TRAY: Mutex<Option<TrayIcon>> = Mutex::new(None);

/// Mirrored WebUI theme: 0 = unobserved yet, 1 = light, 2 = dark.
static MIRRORED: AtomicU8 = AtomicU8::new(0);

/// Last system theme applied to the tray icon (dedupe cache).
static SYS_TRAY: AtomicU8 = AtomicU8::new(0);

const ICON_LIGHT: &[u8] = include_bytes!("../icons/icon-light.ico");
const ICON_DARK: &[u8] = include_bytes!("../icons/icon-dark.ico");

/// Decoded icons are immutable for the process lifetime, so decode once and
/// hand out cheap `Clone`s (tauri's `Image` is `Arc`-backed) instead of
/// re-parsing the embedded `.ico` on every theme switch.
static ICON_LIGHT_CACHE: std::sync::OnceLock<Option<Image<'static>>> = std::sync::OnceLock::new();
static ICON_DARK_CACHE: std::sync::OnceLock<Option<Image<'static>>> = std::sync::OnceLock::new();

/* Installed into the main webview BEFORE page scripts run, so the
   theme the official web app restores at startup is observed instantly
   and mirrored — no shell/webapp double-authority race. */
const THEME_WATCHER_JS: &str = include_str!("../assets/theme-watcher.js");

/* ------------------------------------------------------------------ *
 * Window control commands
 * ------------------------------------------------------------------ */

#[tauri::command]
fn minimize_window(window: WebviewWindow) -> Result<(), String> {
    window.minimize().map_err(|e| e.to_string())
}
#[tauri::command]
fn maximize_window(window: WebviewWindow) -> Result<(), String> {
    window.maximize().map_err(|e| e.to_string())
}
#[tauri::command]
fn toggle_maximize_window(window: WebviewWindow) -> Result<(), String> {
    if window.is_maximized().map_err(|e| e.to_string())? {
        window.unmaximize().map_err(|e| e.to_string())
    } else {
        window.maximize().map_err(|e| e.to_string())
    }
}
#[tauri::command]
fn close_window(window: WebviewWindow) -> Result<(), String> {
    window.app_handle().exit(0);
    Ok(())
}
#[tauri::command]
fn show_window(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_focus();
        let _ = w.show();
    } else if let Some(w) = app.get_webview_window("splash") {
        let _ = w.show();
    }
    Ok(())
}
#[tauri::command]
fn log_engine_event(event: String, payload: String) {
    slog!("[engine-event] {} | {}", event, payload);
}

/* ------------------------------------------------------------------ *
 * Theme mirroring — WebUI is authoritative, native surfaces follow
 * ------------------------------------------------------------------ */

fn code_of(eff: Effective) -> u8 {
    match eff {
        Effective::Dark => 2,
        Effective::Light => 1,
    }
}

/// Decode (once) and return the per-theme tray/window icon.
fn icon_for(eff: Effective) -> Option<Image<'static>> {
    let cache = match eff {
        Effective::Dark => &ICON_DARK_CACHE,
        Effective::Light => &ICON_LIGHT_CACHE,
    };
    cache
        .get_or_init(|| {
            let bytes = match eff {
                Effective::Dark => ICON_DARK,
                Effective::Light => ICON_LIGHT,
            };
            Image::from_bytes(bytes).ok()
        })
        .clone()
}

/* ------------------------------------------------------------------ *
 * Win32 split-icon plumbing (Windows).
 *
 * WM_SETICON has two independent slots:
 *   ICON_SMALL -> title bar corner icon  (WebUI-driven)
 *   ICON_BIG   -> taskbar button / alt-tab icon (system-driven)
 * tao's set_icon overwrites BOTH, so we drive the slots directly.
 * ------------------------------------------------------------------ */

#[cfg(windows)]
static PREV_SMALL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(windows)]
static PREV_BIG: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(windows)]
fn win32_set_icon(hwnd: isize, img: &Image, big: bool) {
    use windows_sys::Win32::Graphics::Gdi::{
        CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER,
        BI_RGB, DIB_RGB_COLORS,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateIconIndirect, DestroyIcon, SendMessageW, ICONINFO, WM_SETICON,
    };

    let (w, h) = (img.width() as i32, img.height() as i32);
    if w == 0 || h == 0 {
        return;
    }
    /* RGBA -> premultiplied BGRA, bottom-up rows for DIB. */
    let src = img.rgba();
    let mut pixels: Vec<u8> = Vec::with_capacity(src.len());
    for y in (0..h as usize).rev() {
        for x in 0..w as usize {
            let idx = (y * w as usize + x) * 4;
            let (r, g, b, a) = (
                src[idx] as u32,
                src[idx + 1] as u32,
                src[idx + 2] as u32,
                src[idx + 3] as u32,
            );
            pixels.push(((b * a + 127) / 255) as u8);
            pixels.push(((g * a + 127) / 255) as u8);
            pixels.push(((r * a + 127) / 255) as u8);
            pixels.push(a as u8);
        }
    }

    unsafe {
        let mut bi: BITMAPINFO = std::mem::zeroed();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w;
        bi.bmiHeader.biHeight = h; /* bottom-up */
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB;
        bi.bmiHeader.biSizeImage = pixels.len() as u32;

        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let hbm_color = CreateDIBSection(
            std::ptr::null_mut(),
            &bi as *const BITMAPINFO as *const _,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );
        if hbm_color.is_null() || bits.is_null() {
            return;
        }
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u8, pixels.len());

        let hbm_mask = CreateBitmap(w, h, 1, 1, std::ptr::null());
        let ii = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: hbm_mask,
            hbmColor: hbm_color,
        };
        let hicon = CreateIconIndirect(&ii);
        let _ = DeleteObject(hbm_mask as _);
        let _ = DeleteObject(hbm_color as _);
        if hicon.is_null() {
            return;
        }

        let slot = if big { &PREV_BIG } else { &PREV_SMALL };
        let prev = slot.swap(hicon as usize, Ordering::SeqCst);
        SendMessageW(
            hwnd as _,
            WM_SETICON,
            if big { 1usize } else { 0usize },
            hicon as isize,
        );
        if prev != 0 {
            let _ = DestroyIcon(prev as _);
        }
    }
}
/// Mirror one observed WebUI theme onto the titlebar surfaces.
///
/// Invariant (spec v2): titlebar matches WebUI at all times. The WebUI
/// owns its state - including its own "follow system" mode - so the
/// shell never decides, never persists, only reflects.
///
/// Scope: DWM titlebar + window icon ONLY. The tray/taskbar follows
/// the OS theme independently (see `apply_system_tray`).
///
/// Titlebar uses the raw DWM attribute instead of Tauri's `set_theme`:
/// the latter propagates into the WebView (prefers-color-scheme), which
/// perturbs the very page we observe and creates a feedback loop.
fn mirror_theme(app: &AppHandle, eff: Effective, sig: &str) {
    let new_code = code_of(eff);
    let changed = MIRRORED.load(Ordering::SeqCst) != new_code;
    if !changed {
        return;
    }
    MIRRORED.store(new_code, Ordering::SeqCst);
    slog!("[theme] webui={} via {} - mirroring titlebar", eff.as_str(), sig);

    /* Titlebar chrome: DWM dark mode on the main window's HWND. */
    #[cfg(windows)]
    {
        use windows_sys::Win32::Graphics::Dwm::{
            DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE,
        };
        if let Some(w) = app.get_webview_window("main") {
            if let Ok(hwnd) = w.hwnd() {
                let val: i32 = matches!(eff, Effective::Dark) as i32;
                unsafe {
                    let _ = DwmSetWindowAttribute(
                        hwnd.0 as _,
                        DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                        &val as *const i32 as *const core::ffi::c_void,
                        4,
                    );
                }
            }
        }
    }

    /* Title bar SMALL icon follows the WebUI theme. The taskbar BIG slot
       is system-driven - see apply_system_tray. */
    #[cfg(windows)]
    {
        if let Some(icon) = icon_for(eff) {
            if let Some(w) = app.get_webview_window("main") {
                if let Ok(hwnd) = w.hwnd() {
                    win32_set_icon(hwnd.0 as isize, &icon, false);
                }
            }
        }
    }
}

/// Tray / taskbar icon tracks the SYSTEM theme - independent from the
/// WebUI mirror chain by design.
fn apply_system_tray(app: &AppHandle, eff: Effective) {
    let code = code_of(eff);
    if SYS_TRAY.load(Ordering::SeqCst) == code {
        return;
    }
    SYS_TRAY.store(code, Ordering::SeqCst);
    let mut swapped = false;
    if let Ok(g) = TRAY.lock() {
        if let Some(tray) = g.as_ref() {
            if let Some(icon) = icon_for(eff) {
                swapped = tray.set_icon(Some(icon)).is_ok();
            }
        }
    }
    /* Taskbar button (BIG slot) tracks the system theme as well. */
    #[cfg(windows)]
    {
        if let Some(icon) = icon_for(eff) {
            if let Some(w) = app.get_webview_window("main") {
                if let Ok(hwnd) = w.hwnd() {
                    win32_set_icon(hwnd.0 as isize, &icon, true);
                }
            }
        }
    }
    if swapped {
        slog!("[theme] system={} - tray icon swapped", eff.as_str());
    }
}
/// Called from the injected watcher whenever the WebUI's theme signal
/// appears or changes.
///
/// `sig` diagnostics prefixed with `diag:` are logged but never mirrored.
#[tauri::command]
fn webui_theme_changed(app: AppHandle, dark: bool, sig: Option<String>) {
    let sig = sig.unwrap_or_else(|| "unknown".into());
    if sig.starts_with("diag:") {
        slog!("[theme-diag] invoke alive: {}", sig);
        return;
    }
    let eff = if dark { Effective::Dark } else { Effective::Light };
    mirror_theme(&app, eff, &sig);
}

/* ------------------------------------------------------------------ *
 * HTTP beacon fallback transport.
 *
 * The watcher prefers Tauri IPC; if `__TAURI__` is unavailable on the
 * external page (ACL/pattern quirks), it beacons via cross-origin
 * `fetch(..., {mode:'no-cors'})` to this listener instead. Works for
 * ANY localhost origin regardless of capability pattern semantics.
 * ------------------------------------------------------------------ */

const BEACON_PORTS: [u16; 10] = [38080, 38081, 38082, 38083, 38084, 38085, 38086, 38087, 38088, 38089];

fn start_theme_beacon(handle: AppHandle) {
    thread::spawn(move || {
        let mut bound = None;
        for port in BEACON_PORTS {
            match std::net::TcpListener::bind(("127.0.0.1", port)) {
                Ok(l) => {
                    slog!("[theme-beacon] listening on {port}");
                    bound = Some((l, port));
                    break;
                }
                Err(_) => continue,
            }
        }
        let Some((listener, _port)) = bound else {
            slog!("[theme-beacon] no port available");
            return;
        };
        use std::io::Read;
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = [0u8; 1024];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            /* Request line: GET /beacon?dark=1&src=... HTTP/1.1 */
            let reply = b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n";
            let _ = std::io::Write::write_all(&mut s, reply);
            if let Some(qs) = req.split_whitespace().nth(1) {
                let dark = qs.contains("dark=1") || qs.contains("dark=true");
                let src = qs
                    .split(|c| c == '&' || c == '?')
                    .find_map(|kv| kv.strip_prefix("sig="))
                    .unwrap_or("beacon")
                    .to_string();
                let eff = if dark { Effective::Dark } else { Effective::Light };
                let src2 = urldecode(&src);
                slog!("[theme-beacon] report {} via {}", eff.as_str(), src2);
                if src2.starts_with("diag:") {
                    continue;
                }
                let h_a = handle.clone();
                let h_b = handle.clone();
                let sig_c = src2;
                let _ = h_a.run_on_main_thread(move || mirror_theme(&h_b, eff, &sig_c));
            }
        }
    });
}

fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

#[tauri::command]
fn get_theme_status() -> String {
    let effective = match MIRRORED.load(Ordering::SeqCst) {
        2 => "dark",
        1 => "light",
        _ => "unobserved",
    };
    format!("{{\"source\":\"webui\",\"effective\":\"{}\"}}", effective)
}

/* ------------------------------------------------------------------ *
 * Engine commands
 * ------------------------------------------------------------------ */

#[tauri::command]
fn start_engine(workdir: Option<String>) -> Result<u16, String> {
    let wd = workdir.map(Into::into);
    {
        let g = HARNESS.lock().map_err(|_| "lock poisoned")?;
        if let Some(h) = g.as_ref() {
            if h.is_running() {
                return h.port();
            }
        }
    }
    let h = HarnessManager::new(wd, None);
    h.start()?;
    let h = Arc::new(h);
    let mut g = HARNESS.lock().map_err(|_| "lock poisoned")?;
    *g = Some(h.clone());
    h.port()
}
#[tauri::command]
fn stop_engine(_app: AppHandle) -> Result<(), String> {
    let mut g = HARNESS.lock().map_err(|_| "lock poisoned")?;
    if let Some(h) = g.as_ref() {
        let _ = h.stop();
    }
    *g = None;
    Ok(())
}
#[tauri::command]
fn is_engine_running() -> bool {
    if let Ok(g) = HARNESS.lock() {
        if let Some(h) = g.as_ref() {
            return h.is_running();
        }
    }
    false
}
#[tauri::command]
fn engine_port() -> Result<u16, String> {
    if let Ok(g) = HARNESS.lock() {
        if let Some(h) = g.as_ref() {
            return h.port();
        }
    }
    Err("engine not running".to_string())
}
#[tauri::command]
fn engine_status(_app: AppHandle) -> String {
    let running = is_engine_running();
    let port = if running {
        engine_port().ok().unwrap_or(0)
    } else {
        0
    };
    if running {
        format!("{{\"running\":true,\"port\":{}}}", port)
    } else {
        "{\"running\":false}".to_string()
    }
}

/// Resolve the engine as `(node_exe, dsh_bin_js)`.
///
/// Search order (first hit wins):
///  1. the Tauri resource dir — `<resource_dir>/engine` (the production /
///     installed layout; both Node and the engine ship here so the user needs
///     no separate Node install),
///  2. the app data dir (`engine/`),
///  3. the current working directory (dev / `tauri dev`),
///  4. walking up from the executable's own directory (covers double-clicked
///     builds whose cwd is their own folder).
///
/// `node` defaults to the system `node` on `PATH` unless a bundled `node.exe`
/// is found alongside the engine.
fn resolve_engine(app: &AppHandle) -> Option<(PathBuf, PathBuf)> {
    let mut roots: Vec<PathBuf> = Vec::new();

    if let Ok(rd) = app.path().resource_dir() {
        roots.push(rd.join("engine"));
    }
    if let Ok(data_dir) = app.path().app_data_dir() {
        roots.push(data_dir.join("engine"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent();
        while let Some(d) = dir {
            roots.push(d.to_path_buf());
            dir = d.parent();
        }
    }

    /* Each root may host the engine either directly (`<root>/node_modules`)
       or under an `engine/` subfolder (`<root>/engine/node_modules`), so both
       the dev layout and the bundled/portable layout resolve. */
    let mut bases: Vec<PathBuf> = Vec::with_capacity(roots.len() * 2);
    for r in &roots {
        bases.push(r.clone());
        bases.push(r.join("engine"));
    }

    let mut node = PathBuf::from("node");
    for b in &bases {
        let n = b.join("node.exe");
        if n.exists() {
            node = n;
            break;
        }
    }

    for b in &bases {
        let bin = b
            .join("node_modules")
            .join("@deepseek-ai")
            .join("dsh")
            .join("lib")
            .join("bin.js");
        if bin.exists() {
            return Some((node.clone(), bin));
        }
    }
    None
}

/* ------------------------------------------------------------------ *
 * Windows
 * ------------------------------------------------------------------ */

/// Build the (single) main window.
///
/// * `port = None`  -> show the local "starting engine" loading page
///   immediately, so the user gets instant feedback instead of a blank gap
///   while the engine cold-starts.
/// * `port = Some(p)` -> point straight at the engine's Web UI.
fn build_main_window(app: &AppHandle, port: Option<u16>) -> Result<(), String> {
    if app.get_webview_window("main").is_some() {
        return Ok(());
    }
    let url = match port {
        Some(p) => WebviewUrl::External(
            url::Url::parse(&format!("http://localhost:{}", p)).map_err(|e| e.to_string())?,
        ),
        None => WebviewUrl::App("assets/loading.html".into()),
    };
    WebviewWindowBuilder::new(app, "main", url)
        .inner_size(1024.0, 680.0)
        .min_inner_size(800.0, 560.0)
        .center()
        .title("DeepSeek Harness")
        .build()
        .map_err(|e| e.to_string())?;

    /* NOTE: no set_theme here on purpose. Pinning would freeze the
       WebView's prefers-color-scheme at launch time, breaking the
       WebUI's own "follow system" mode (Bug A) and suppressing the
       ThemeChanged events the tray relies on (Bug B). The window stays
       unpinned: native chrome follows the OS until the first watcher
       report, after which DWM keeps the titlebar WebUI-driven. */
    slog!("[shell] main window built OK");

    /* Inject the theme watcher through the host->webview eval channel.
       Self-healing: re-eval every 5s forever. The script is install-once
       guarded inside the page, so repeats are no-ops; this survives SPA
       navigations, transient failures, and timing races alike. */
    {
        let app2 = app.clone();
        thread::spawn(move || {
            let mut logged = 0;
            loop {
                /* Chain proven alive once first observation mirrored. */
                if MIRRORED.load(Ordering::SeqCst) != 0 {
                    break;
                }
                if let Some(w) = app2.get_webview_window("main") {
                    match w.eval(THEME_WATCHER_JS) {
                        Ok(()) => {
                            if logged < 3 {
                                slog!("[theme-inject] eval ok #{}", logged + 1);
                                logged += 1;
                            }
                        }
                        Err(e) => {
                            if logged < 10 {
                                slog!("[theme-inject] eval ERR: {e}");
                                logged += 1;
                            }
                        }
                    }
                } else if logged < 10 {
                    slog!("[theme-inject] main window not found");
                    logged += 1;
                }
                thread::sleep(Duration::from_secs(5));
            }
        });
    }

    /* Close the splash window now that the main window is up. */
    if let Some(splash) = app.get_webview_window("splash") {
        let _ = splash.close();
    }

    Ok(())
}

fn build_tray_menu(app: &AppHandle) -> tauri::Result<()> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem};
    use tauri::tray::TrayIconBuilder;

    let menu = MenuBuilder::new(app)
        .item(&MenuItemBuilder::with_id("show", "Show Workspace").build(app)?)
        .item(&MenuItemBuilder::with_id("stop_engine", "Stop Engine").build(app)?)
        .item(&MenuItemBuilder::with_id("settings", "Settings...").build(app)?)
        .separator()
        .item(&MenuItemBuilder::with_id("about", "About DeepSeek Harness").build(app)?)
        .item(&PredefinedMenuItem::quit(app, Some("Quit"))?)
        .build()?;

    /* Initial guess before any WebUI observation: current OS theme. */
    let mut builder = TrayIconBuilder::with_id("main-tray")
        .menu(&menu)
        .tooltip("DeepSeek Harness")
        .show_menu_on_left_click(true);
    if let Some(icon) = icon_for(theme::system_chrome_theme()) {
        builder = builder.icon(icon);
    }

    let tray = builder
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "show" => {
                let _ = show_window(app.clone());
            }
            "stop_engine" => {
                let _ = stop_engine(app.clone());
                slog!("[tray] engine stopped");
            }
            "settings" => {
                #[allow(deprecated)]
                {
                    use tauri_plugin_shell::ShellExt;
                    let _ = app.shell().open(
                        "https://deepseek-harness.github.io/docs/",
                        None::<tauri_plugin_shell::open::Program>,
                    );
                }
                slog!("[tray] opening settings docs");
            }
            "about" => {
                slog!("[tray] about: DeepSeek Harness Desktop v0.1.0");
            }
            "quit" | "quit_menu" => {
                let _ = stop_engine(app.clone());
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;

    *TRAY.lock().expect("lock poisoned") = Some(tray);

    Ok(())
}

pub fn run() {
    let _ = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            /* File logger FIRST — every later diagnostic survives any
               launch mode (double-click, redirected, background). */
            let log_dir = app
                .handle()
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::env::temp_dir())
                .join("logs");
            logging::init(log_dir);
            slog!("[shell] Tauri 2 shell v0.1.0");

            let result = build_tray_menu(app.handle());
            if let Err(e) = result {
                slog!("[tray] build failed: {}", e);
            }

            start_theme_beacon(app.handle().clone());

            /* Seed the tray icon from the current OS *chrome* theme. */
            apply_system_tray(app.handle(), theme::system_chrome_theme());

            /* Bug B self-healing: poll the REGISTRY (authoritative OS
               app-mode state) every 5s and keep tray + taskbar BIG icon
               in sync. Never query the window here: its tracked theme is
               poisoned by our own DWM titlebar override. Cheap: one
               `reg query` (~15ms) per tick, deduped downstream. */
            {
                let app4 = app.handle().clone();
                thread::spawn(move || loop {
                    thread::sleep(Duration::from_secs(5));
                    let eff = theme::system_chrome_theme();
                    apply_system_tray(&app4, eff);
                    let _ = &app4;
                });
            }

            /* Show the main window IMMEDIATELY with a local "starting engine"
               loading page, so clicking the exe gives instant feedback (no
               blank gap). Once the engine is reachable we navigate this same
               window to the Web UI — still a single window, never a second
               popup. */

            // open the window right away (loading page)
            if let Err(e) = build_main_window(app.handle(), None) {
                slog!("[shell] initial main window build failed: {e}");
            }

            let workdir = app.path().app_data_dir().ok();
            let engine = resolve_engine(app.handle());
            let handle = app.handle().clone();

            thread::spawn(move || {
                let h = HarnessManager::new(workdir, engine);
                let handle_run = handle.clone();

                let port = match h.start() {
                    Ok(()) => h.port().ok().unwrap_or(3080),
                    Err(err) => {
                        slog!("[shell] engine start failed: {err}");
                        let hr = handle_run.clone();
                        let err_msg = err;
                        let _ = handle_run.run_on_main_thread(move || {
                            if let Some(w) = hr.get_webview_window("main") {
                                let _ = w.eval(&format!(
                                    "alert('Engine failed to start: {err_msg}')"
                                ));
                            }
                        });
                        return;
                    }
                };
                let h = Arc::new(h);
                *HARNESS.lock().expect("lock poisoned") = Some(h.clone());

                // navigate the already-open window to the Web UI
                let handle_run2 = handle_run.clone();
                let nav_url = format!("http://localhost:{}", port);
                let _ = handle_run.run_on_main_thread(move || {
                    if let Some(w) = handle_run2.get_webview_window("main") {
                        // advance the loading UI to the final stage, then
                        // hand off this same window to the Web UI.
                        let _ = w.eval(
                            "window.__dsh_loading && window.__dsh_loading.set(3,'加载工作区…')",
                        );
                        match url::Url::parse(&nav_url) {
                            Ok(u) => {
                                let _ = w.navigate(u);
                                slog!("[shell] main window navigated to {nav_url}");
                            }
                            Err(e) => slog!("[shell] nav url parse err: {e}"),
                        }
                    } else {
                        // window not open yet (shouldn't happen) — build it
                        if build_main_window(&handle_run2, Some(port)).is_err() {
                            slog!("[shell] main window build failed");
                        }
                    }
                });
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            minimize_window,
            maximize_window,
            toggle_maximize_window,
            close_window,
            show_window,
            log_engine_event,
            start_engine,
            stop_engine,
            is_engine_running,
            engine_port,
            engine_status,
            webui_theme_changed,
            get_theme_status,
        ])
        .build(tauri::generate_context!())
        .expect("Failed to build Tauri app")
        .run(|_app_handle, event| match event {
            RunEvent::Exit => {
                if let Ok(g) = HARNESS.lock() {
                    if let Some(h) = g.as_ref() {
                        let _ = h.stop();
                    }
                }
            }
            /* OS theme flipped: tray/taskbar follows the *system chrome*
                theme (taskbar background), independent of the WebUI mirror
                chain. Re-read the registry rather than trust the event's
                payload, because tao reports the app-mode theme which can
                diverge from the taskbar background we must stay visible on. */
            RunEvent::WindowEvent { event, .. } => {
                if let tauri::WindowEvent::ThemeChanged(_t) = event {
                    apply_system_tray(_app_handle, theme::system_chrome_theme());
                }
            }
            _ => {}
        });
}
