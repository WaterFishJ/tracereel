//! Diagnostic CLI; the GUI calls the library directly with a cached Engine.
use std::io::Read;
fn main() {
    let result = (|| -> Result<(), String> {
        let mut input = String::new();
        std::io::stdin()
            .read_to_string(&mut input)
            .map_err(|e| e.to_string())?;
        let request = serde_json::from_str(&input).map_err(|e| e.to_string())?;
        let start = std::time::Instant::now();
        let mut engine = receipt_ocr::Engine::new(&receipt_ocr::runtime_dir()?)?;
        eprintln!("models loaded: {:?}", start.elapsed());
        let passes = if std::env::args().any(|a| a == "--benchmark") {
            2
        } else {
            1
        };
        for pass in 0..passes {
            let began = std::time::Instant::now();
            let result = engine.recognize(&request)?;
            eprintln!("recognition {pass}: {:?}", began.elapsed());
            if pass == 0 {
                println!(
                    "{}",
                    serde_json::to_string(&result).map_err(|e| e.to_string())?
                );
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
