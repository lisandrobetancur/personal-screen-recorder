//! Ícono en la barra de menú (bandeja del sistema en Windows) y atajo global
//! para grabar y detener sin abrir la ventana. La grabación la arranca y la
//! detiene la interfaz (que conoce las opciones elegidas): aquí solo se le
//! avisa con el evento `toggle-recording`.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Emitter, Manager, Wry};

/// Atajo global: ⌘⇧9 en Mac, Ctrl+Shift+9 en Windows.
pub const SHORTCUT: &str = "CommandOrControl+Shift+9";

const IDLE_ICON: &[u8] = include_bytes!("../icons/tray/idle.png");
const RECORDING_ICON: &[u8] = include_bytes!("../icons/tray/recording.png");

pub struct Tray {
    icon: TrayIcon,
    toggle: MenuItem<Wry>,
    started: Mutex<Option<Instant>>,
}

/// Texto del atajo para mostrarlo en la interfaz.
pub fn shortcut_label() -> &'static str {
    if cfg!(target_os = "macos") { "⌘⇧9" } else { "Ctrl+Shift+9" }
}

fn toggle_label(recording: bool) -> &'static str {
    if recording { tr!("Detener grabación", "Stop recording") } else { tr!("Grabar", "Record") }
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let toggle = MenuItem::with_id(app, "toggle", toggle_label(false), true, Some(SHORTCUT))?;
    let show = MenuItem::with_id(app, "show", tr!("Mostrar la app", "Show the app"), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", tr!("Salir", "Quit"), true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&toggle, &show, &separator, &quit])?;

    let icon = TrayIconBuilder::with_id("main")
        .icon(Image::from_bytes(IDLE_ICON)?)
        // En reposo es una plantilla: macOS la pinta clara u oscura según el tema.
        .icon_as_template(true)
        .tooltip("Personal Screen Recorder")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "toggle" => request_toggle(app),
            "show" => show_window(app),
            "quit" => crate::quit(app),
            _ => {}
        })
        .build(app)?;

    app.manage(Tray { icon, toggle, started: Mutex::new(None) });

    // Cronómetro junto al ícono mientras se graba.
    let handle = app.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(500));
        if let Some(tray) = handle.try_state::<Tray>() {
            tray.tick();
        }
    });
    Ok(())
}

/// Pide a la interfaz que empiece o detenga la grabación.
pub fn request_toggle(app: &AppHandle) {
    let _ = app.emit("toggle-recording", ());
}

pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

impl Tray {
    pub fn set_recording(&self, recording: bool) {
        if let Ok(mut started) = self.started.lock() {
            *started = recording.then(Instant::now);
        }
        let _ = self.toggle.set_text(toggle_label(recording));
        let image = Image::from_bytes(if recording { RECORDING_ICON } else { IDLE_ICON });
        if let Ok(image) = image {
            let _ = self.icon.set_icon(Some(image));
        }
        // El punto rojo debe verse rojo: no es plantilla.
        let _ = self.icon.set_icon_as_template(!recording);
        if !recording {
            let _ = self.icon.set_title(None::<&str>);
            let _ = self.icon.set_tooltip(Some("Personal Screen Recorder"));
        }
    }

    fn tick(&self) {
        let Some(started) = self.started.lock().ok().and_then(|s| *s) else { return };
        let total = started.elapsed().as_secs();
        let elapsed = format!("{:02}:{:02}:{:02}", total / 3600, total / 60 % 60, total % 60);
        // En Mac se ve junto al ícono; en Windows, al pasar el puntero.
        let _ = self.icon.set_title(Some(&elapsed));
        let _ = self.icon.set_tooltip(Some(format!("{} — {elapsed}", tr!("Grabando", "Recording"))));
    }
}
