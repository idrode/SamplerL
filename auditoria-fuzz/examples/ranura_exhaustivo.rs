//! El protocolo de `engine::device::EngineSlot` (tomar el puntero con
//! `swap(null)`, usarlo y devolverlo con `store`), con todos los
//! entrelazados posibles de tres hilos (dos callbacks de dos bloques y el
//! hilo de control), cada operación atómica como un paso. Sin dependencias.
//!
//! Comprueba, en cada estado: nunca hay dos hilos con el motor a la vez; y
//! al acabar, el puntero está en la ranura.
//!
//! Vale para una memoria secuencialmente consistente. Para la memoria débil
//! el argumento es de lectura: un `swap` es una operación de
//! lectura-modificación-escritura, que lee siempre el último valor del
//! orden de modificaciones, así que una ranura con el puntero no puede dar
//! nulo y nadie escribe el puntero sin haberlo tomado antes. `loom` 0.7.3
//! (examples/loom_ranura.rs) no sirve aquí: da por perdido el puntero hasta
//! en un modelo mínimo de dos hilos con todo `SeqCst` (examples/loom_min.rs),
//! una ejecución que ese mismo argumento prohíbe.
//!
//!     cargo run --release --example ranura_exhaustivo

#[derive(Clone, Copy, PartialEq)]
enum Fase {
    Tomar,
    Devolver,
    Fin,
}

#[derive(Clone)]
struct Estado {
    ranura: bool, // el puntero está en la ranura
    fase: [Fase; 3],
    tiene: [bool; 3],
    quedan: [u8; 3],
}

fn explorar(e: &Estado, estados: &mut u64, finales: &mut u64) {
    *estados += 1;
    assert!(e.tiene.iter().filter(|&&t| t).count() <= 1, "dos hilos con el motor a la vez");
    assert!(e.ranura != e.tiene.iter().any(|&t| t), "el puntero está en dos sitios o en ninguno");
    let mut alguno = false;
    for h in 0..3 {
        let mut n = e.clone();
        match e.fase[h] {
            Fase::Fin => continue,
            Fase::Tomar => {
                // swap(null): se lleva lo que haya.
                n.tiene[h] = e.ranura;
                n.ranura = false;
                n.fase[h] = if n.tiene[h] { Fase::Devolver } else { siguiente(&mut n, h) };
            }
            Fase::Devolver => {
                // store(p)
                n.ranura = true;
                n.tiene[h] = false;
                n.fase[h] = siguiente(&mut n, h);
            }
        }
        alguno = true;
        explorar(&n, estados, finales);
    }
    if !alguno {
        *finales += 1;
        assert!(e.ranura, "el motor se perdió");
    }
}

fn siguiente(e: &mut Estado, h: usize) -> Fase {
    e.quedan[h] -= 1;
    if e.quedan[h] == 0 { Fase::Fin } else { Fase::Tomar }
}

fn main() {
    let inicio = Estado { ranura: true, fase: [Fase::Tomar; 3], tiene: [false; 3], quedan: [3, 3, 2] };
    let (mut estados, mut finales) = (0, 0);
    explorar(&inicio, &mut estados, &mut finales);
    println!("{estados} estados y {finales} entrelazados completos: nunca dos hilos con el motor, y el puntero siempre vuelve");
}
