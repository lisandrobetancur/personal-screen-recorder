// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--autoprueba") {
        let seconds = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(5);
        let microphone = args.iter().any(|a| a == "--mic");
        match personal_screen_recorder_lib::self_test(seconds, microphone) {
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
