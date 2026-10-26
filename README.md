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
  cada uno en **su propia pista**. Antes de codificar, todo el audio se lleva a 48 kHz
  con tiempos continuos (los audífonos Bluetooth cambian de formato al activar el
  micrófono) y, si no suena nada, la pista se rellena con silencio.
- Mientras graba se escribe un **.mov fragmentado cada 2 s**: si la app se cierra de
  golpe, ese archivo sigue sirviendo hasta el último fragmento. Al detener se copia a
  **.mp4 sin recomprimir** (segundos) y se borra el .mov. Si la copia fallara, queda el .mov.
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

### Firma y permisos en macOS

La app se firma con el certificado local **«PSR Desarrollo»** (Acceso a Llaveros →
Asistente para certificados → Crear un certificado: raíz autofirmada, tipo *Firma de
código*). Con una firma estable, macOS conserva los permisos entre compilaciones; con
la firma provisional (ad-hoc) los pedía de nuevo cada vez.

La primera vez la app pide **Grabación de pantalla y audio del sistema** y, al grabar
con micrófono, **Micrófono**. Tras conceder el de pantalla hay que reabrir la app. Para
empezar de cero con el permiso de pantalla:

```bash
tccutil reset ScreenCapture com.co.personal-screen-recorder
```

`src-tauri/Entitlements.plist` declara micrófono y cámara: con *hardened runtime*, sin
eso macOS los niega sin mostrar aviso.

## Estructura

- `src/`: interfaz (HTML, CSS y TypeScript).
- `src-tauri/src/lib.rs`: comandos que la interfaz llama.
- `src-tauri/src/capture/`: motor de grabación. `macos.rs` usa ScreenCaptureKit y AVAssetWriter;
  `unsupported.rs` es el marcador mientras no exista el motor de Windows.
