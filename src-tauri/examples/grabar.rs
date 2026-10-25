//! Prueba del motor sin interfaz: graba la primera pantalla durante N
//! segundos (por omisión 5). Necesita el permiso de pantalla para la terminal.
//!
//!     cargo run --example grabar -- 10 --mic

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let seconds = args.iter().find_map(|a| a.parse().ok()).unwrap_or(5);
    let microphone = args.iter().any(|a| a == "--mic");
    let path = personal_screen_recorder_lib::self_test(seconds, microphone)?;
    println!("Listo: {}", path.display());
    Ok(())
}
