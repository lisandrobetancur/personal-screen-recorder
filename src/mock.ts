// Datos de ejemplo para revisar la interfaz en un navegador común
// (`pnpm dev` y abrir http://localhost:1420). Solo se usa fuera de Tauri en desarrollo.

const sources = [
  { kind: "display", id: 1, name: "Pantalla 1 — 3456×2234", width: 3456, height: 2234 },
  { kind: "display", id: 2, name: "Pantalla 2 — 2560×1440", width: 2560, height: 1440 },
  { kind: "window", id: 10, name: "Google Chrome — Clase 4: Fisiología respiratoria", width: 1920, height: 1080 },
  { kind: "window", id: 11, name: "Zoom — Reunión semanal", width: 1600, height: 1000 },
  { kind: "window", id: 12, name: "Keynote — Presentación final.key", width: 2560, height: 1600 },
];

const microphones = [
  { id: "a", name: "Micrófono de MacBook Pro", isDefault: true },
  { id: "b", name: "WH-XB910N", isDefault: false },
];

export async function mockPickFolder(): Promise<string> {
  return "/Users/demo/Documents/Clases grabadas";
}

export async function mockInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  await new Promise((r) => setTimeout(r, 150));
  const results: Record<string, unknown> = {
    list_sources: sources,
    list_microphones: microphones,
    get_output_dir: "/Users/demo/Movies/Personal Screen Recorder",
    set_output_dir: args?.path,
    start_recording: "/Users/demo/Movies/Personal Screen Recorder/Grabación 2026-10-25 17.02.11.mp4",
    stop_recording: "/Users/demo/Movies/Personal Screen Recorder/Grabación 2026-10-25 17.02.11.mp4",
  };
  return results[cmd] as T;
}
