//! Reescribe la sección de la API de `docs/GUIA.md` a partir del registro
//! (`script::api`): `cargo run -p script --example guia`.

fn main() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/GUIA.md");
    let guide = std::fs::read_to_string(path).expect("no se pudo leer docs/GUIA.md");
    let updated = script::api::update_guide(&guide)
        .expect("faltan las marcas api:inicio / api:fin en docs/GUIA.md");
    if updated == guide {
        println!("docs/GUIA.md ya está al día");
    } else {
        std::fs::write(path, updated).expect("no se pudo escribir docs/GUIA.md");
        println!("docs/GUIA.md actualizada");
    }
}
