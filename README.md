# Personal Screen Recorder

Grabador de pantalla y audio para macOS (Intel y Apple Silicon) y Windows, hecho con
Rust + Tauri 2.

## Estado

| Función | macOS | Windows |
|---|---|---|
| Pantalla completa | ✅ | ⏳ |
| Ventana | ✅ | ⏳ |
| Audio del sistema | ✅ (13+) | ⏳ |
| Micrófono | ✅ (15+) | ⏳ |
| Pestaña del navegador (extensión) | ⏳ | ⏳ |
| Varias pantallas a la vez | ⏳ | ⏳ |
| Cámara web | ⏳ | ⏳ |
| Pista de audio mezclada | ⏳ | ⏳ |

## Formato de salida

- **MP4**, video **H.264** por hardware (HEVC si la fuente supera 4096×2304).
- **Audio AAC 48 kHz**: audio del sistema en estéreo a 320 kbps y micrófono a 192 kbps,
  cada uno en **su propia pista**.
- Se escribe **fragmentado cada 2 s**: si la app se cierra de golpe, el archivo sigue
  sirviendo hasta el último fragmento.
- Se guarda en `~/Movies/Personal Screen Recorder` (Mac) o `Videos\Personal Screen Recorder` (Windows).

## Desarrollo

Requisitos: Rust (`brew install rustup`), Node y pnpm, Xcode.

```bash
pnpm install
pnpm tauri dev                    # app en modo desarrollo
pnpm tauri build --bundles app    # .app en src-tauri/target/release/bundle/macos
```

Probar el motor sin interfaz, con los permisos de la app instalada (graba la primera
pantalla N segundos e imprime la ruta o el error):

```bash
open -n -W --stdout /tmp/prueba.txt "src-tauri/target/release/bundle/macos/Personal Screen Recorder.app" --args --autoprueba 6
```

Revisar el diseño en un navegador común, con datos de ejemplo: `pnpm dev` y abrir
http://localhost:1420.

### Permisos en macOS

La primera vez, macOS pide **Grabación de pantalla y audio del sistema** y **Micrófono**.
Se activan en Ajustes del Sistema → Privacidad y seguridad, y luego hay que reabrir la app.
El permiso se concede a la app (o a la terminal desde la que corre `tauri dev`). Como la
app tiene una firma local (ad-hoc), al recompilarla macOS puede volver a pedirlo.

## Estructura

- `src/`: interfaz (HTML, CSS y TypeScript).
- `src-tauri/src/lib.rs`: comandos que la interfaz llama.
- `src-tauri/src/capture/`: motor de grabación. `macos.rs` usa ScreenCaptureKit y AVAssetWriter;
  `unsupported.rs` es el marcador mientras no exista el motor de Windows.
