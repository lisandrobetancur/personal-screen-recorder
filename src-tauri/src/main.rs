// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--autoprueba") {
        let seconds = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(5);
        let microphone = args.iter().any(|a| a == "--mic");
        let system_audio = !args.iter().any(|a| a == "--sin-audio");
        let value = |flag: &str, default: u32| {
            args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)?.parse().ok()).unwrap_or(default)
        };
        let (fps, display) = (value("--fps", 30), value("--pantalla", 1) as usize);
        match personal_screen_recorder_lib::self_test(seconds, display, fps, system_audio, microphone) {
            Ok(path) => println!("OK {}", path.display()),
            Err(e) => {
                println!("ERROR {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    personal_screen_recorder_lib::run()
}
