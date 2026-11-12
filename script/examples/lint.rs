//! El linter del editor sobre archivos: `cargo run -p script --example lint
//! -- canción.lua [otra.lua ...]`. Escribe cada diagnóstico como
//! `archivo:línea:columna: gravedad: mensaje` y sale con 1 si hay alguno.

fn main() {
    let mut any = false;
    for path in std::env::args().skip(1) {
        let source = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{path}: {e}");
                any = true;
                continue;
            }
        };
        for lint in script::lint::check(&source, script::DEFAULT_MEMORY_LIMIT) {
            any = true;
            println!(
                "{path}:{}:{}: {}: {}",
                lint.row + 1,
                lint.cols.start + 1,
                lint.severity.label(),
                lint.message
            );
        }
    }
    std::process::exit(i32::from(any));
}
