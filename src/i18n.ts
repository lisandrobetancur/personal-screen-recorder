// Textos de la interfaz en español e inglés. El idioma lo decide la app
// según el sistema (español si el sistema está en español; si no, inglés).

const es = {
  "theme.system": "Tema del sistema",
  "theme.light": "Tema claro",
  "theme.dark": "Tema oscuro",
  "theme.group": "Tema",
  "section.source": "Qué grabar",
  "section.audio": "Audio",
  "section.video": "Video",
  "source.display": "Pantalla",
  "source.window": "Ventana",
  "source.refresh": "Actualizar lista",
  "source.list": "Fuentes",
  "source.noDisplays": "No se encontraron pantallas.",
  "source.noWindows": "No hay ventanas abiertas.",
  "source.choose": "Elige una fuente",
  "audio.system": "Audio del sistema",
  "audio.systemHint": "Lo que suena en el equipo",
  "audio.mic": "Micrófono",
  "audio.micHint": "Tu voz, en una pista aparte",
  "audio.default": "predeterminado",
  "audio.none": "Sin audio",
  "audio.summarySystem": "Sistema",
  "audio.summaryMic": "Micrófono",
  "audio.warn": "Avisar si falta el sonido",
  "audio.warnHint": "Pregunta si no llega audio al grabar",
  "video.fps": "Cuadros por segundo",
  "video.cursor": "Mostrar el cursor",
  "video.clicks": "Resaltar los clics",
  "video.hideSelf": "Ocultar esta app",
  "video.hideSelfHint": "Al grabar una pantalla completa",
  "last.reveal": "Mostrar en la carpeta",
  "last.duration": "Duración",
  "folder.change": "Cambiar",
  "folder.tooltip": "Se guarda en {dir}. Clic para elegir otra carpeta.",
  "folder.pickerTitle": "Carpeta para las grabaciones",
  "folder.changed": "Las próximas grabaciones se guardarán en esa carpeta.",
  "record.start": "Grabar",
  "record.stop": "Detener",
  "record.saving": "Guardando…",
  "record.shortcut": "Grabar o detener desde cualquier app: {shortcut}",
  "status.recording": "Grabando: {name}",
  "status.savingFile": "Guardando el archivo…",
  "status.saved": "Grabación guardada.",
  "alert.real": "Sí, es real",
  "alert.stop": "Detener",
  "alert.blackTitle": "La imagen está en negro",
  "alert.blackText": "Puede ser contenido protegido o la pantalla equivocada. ¿Es negro de verdad?",
  "alert.micTitle": "No llega sonido del micrófono",
  "alert.micText": "Revisa que esté encendido y que sea el correcto. ¿Es silencio real?",
  "alert.systemTitle": "No llega audio del sistema",
  "alert.systemText": "Si todavía no suena nada, es normal. ¿Es silencio real?",
  "alert.micLostTitle": "Se perdió el sonido del micrófono",
  "alert.micLostText": "Hace 15 segundos que no llega nada. ¿Se desconectó o es silencio real?",
  "alert.systemLostTitle": "Se perdió el audio del sistema",
  "alert.systemLostText": "Hace 15 segundos que no suena nada en el equipo. ¿Es silencio real?",
};

type Key = keyof typeof es;

const en: Record<Key, string> = {
  "theme.system": "System theme",
  "theme.light": "Light theme",
  "theme.dark": "Dark theme",
  "theme.group": "Theme",
  "section.source": "What to record",
  "section.audio": "Audio",
  "section.video": "Video",
  "source.display": "Display",
  "source.window": "Window",
  "source.refresh": "Refresh list",
  "source.list": "Sources",
  "source.noDisplays": "No displays found.",
  "source.noWindows": "No open windows.",
  "source.choose": "Choose a source",
  "audio.system": "System audio",
  "audio.systemHint": "Whatever plays on this computer",
  "audio.mic": "Microphone",
  "audio.micHint": "Your voice, on its own track",
  "audio.default": "default",
  "audio.none": "No audio",
  "audio.summarySystem": "System",
  "audio.summaryMic": "Microphone",
  "audio.warn": "Warn if sound is missing",
  "audio.warnHint": "Asks when no audio arrives while recording",
  "video.fps": "Frames per second",
  "video.cursor": "Show the cursor",
  "video.clicks": "Highlight clicks",
  "video.hideSelf": "Hide this app",
  "video.hideSelfHint": "When recording a full display",
  "last.reveal": "Show in folder",
  "last.duration": "Duration",
  "folder.change": "Change",
  "folder.tooltip": "Saved to {dir}. Click to choose another folder.",
  "folder.pickerTitle": "Folder for recordings",
  "folder.changed": "New recordings will be saved to that folder.",
  "record.start": "Record",
  "record.stop": "Stop",
  "record.saving": "Saving…",
  "record.shortcut": "Record or stop from any app: {shortcut}",
  "status.recording": "Recording: {name}",
  "status.savingFile": "Saving the file…",
  "status.saved": "Recording saved.",
  "alert.real": "Yes, it's real",
  "alert.stop": "Stop",
  "alert.blackTitle": "The image is black",
  "alert.blackText": "It may be protected content or the wrong display. Is it really black?",
  "alert.micTitle": "No sound from the microphone",
  "alert.micText": "Check that it's on and that it's the right one. Is the silence real?",
  "alert.systemTitle": "No system audio",
  "alert.systemText": "If nothing is playing yet, that's normal. Is the silence real?",
  "alert.micLostTitle": "Microphone sound was lost",
  "alert.micLostText": "Nothing has arrived for 15 seconds. Was it disconnected, or is the silence real?",
  "alert.systemLostTitle": "System audio was lost",
  "alert.systemLostText": "Nothing has played on this computer for 15 seconds. Is the silence real?",
};

let current: Record<Key, string> = es;

export function setLanguage(code: string) {
  current = code === "es" ? es : en;
  document.documentElement.lang = code === "es" ? "es" : "en";
}

/** Texto traducido; `{nombre}` se reemplaza con `vars.nombre`. */
export function t(key: Key, vars: Record<string, string> = {}) {
  return current[key].replace(/\{(\w+)\}/g, (_, name: string) => vars[name] ?? "");
}

/** Aplica los textos a los elementos marcados con data-i18n, data-i18n-title y data-i18n-label. */
export function translatePage() {
  document.querySelectorAll<HTMLElement>("[data-i18n]").forEach((el) => {
    el.textContent = t(el.dataset.i18n as Key);
  });
  document.querySelectorAll<HTMLElement>("[data-i18n-title]").forEach((el) => {
    el.title = t(el.dataset.i18nTitle as Key);
  });
  document.querySelectorAll<HTMLElement>("[data-i18n-label]").forEach((el) => {
    el.setAttribute("aria-label", t(el.dataset.i18nLabel as Key));
  });
}
