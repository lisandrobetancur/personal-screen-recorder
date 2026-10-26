//! Prueba del motor sin interfaz: graba la primera pantalla durante N
//! segundos (por omisión 5). En Mac necesita el permiso de pantalla para la
//! terminal; en Windows sirve en GitHub Actions (con `--sin-audio`, porque
//! esos equipos no tienen tarjeta de sonido).
//!
//!     cargo run --example grabar -- 10 --mic
//!     cargo run --example grabar -- 5 --sin-audio

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let seconds = args.iter().find_map(|a| a.parse().ok()).unwrap_or(5);
    let microphone = args.iter().any(|a| a == "--mic");
    let system_audio = !args.iter().any(|a| a == "--sin-audio");
    let path = personal_screen_recorder_lib::self_test(seconds, 1, 30, system_audio, microphone)?;
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    println!("Listo: {} ({size} bytes)", path.display());
    if size == 0 {
        return Err("El archivo quedó vacío.".into());
    }
    Ok(())
}
