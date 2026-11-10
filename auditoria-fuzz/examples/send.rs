//! ¿`Engine` es `Send`? `EngineSlot` lo guarda tras un `AtomicPtr`, que es
//! `Send + Sync` sea cual sea su contenido: el compilador no lo comprueba
//! ahí. Si esto compila, pasar el motor al hilo del callback es correcto.
fn es_send<T: Send>() {}

fn main() {
    es_send::<engine::Engine>();
    es_send::<engine::Controller>();
    println!("Engine y Controller son Send");
}
