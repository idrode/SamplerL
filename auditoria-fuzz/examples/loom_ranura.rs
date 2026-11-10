//! Modelo en `loom` de la ranura del motor (`engine/src/device.rs`,
//! `EngineSlot`). **No concluyente**: `loom` 0.7.3 da por perdido el
//! puntero, pero también en un modelo mínimo de dos hilos con todo `SeqCst`
//! (examples/loom_min.rs), y esa ejecución no es posible: un `swap` lee
//! siempre el último valor del orden de modificaciones, así que el `store`
//! del hilo que tenía el puntero no puede quedar por detrás de un `swap`
//! que leyó nulo. Se deja como constancia de lo que se probó; la
//! comprobación que vale es examples/ranura_exhaustivo.rs.
//!
//!     cargo run --release --example loom_ranura   (falla, ver arriba)

use loom::cell::UnsafeCell;
use loom::sync::Arc;
use loom::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use loom::thread;
use std::ptr;

static TRAZA: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

struct Motor {
    bloques: UnsafeCell<usize>,
}

struct Ranura {
    motor: AtomicPtr<Motor>,
    silencios: AtomicUsize,
}

impl Ranura {
    /// Lo que hace `Renderer::render` y `EngineSlot::with`.
    fn usar(&self) -> bool {
        let p = self.motor.swap(ptr::null_mut(), Ordering::Acquire);
        TRAZA.lock().unwrap().push(format!("{:?} toma {}", loom::thread::current().id(), if p.is_null() { "nulo" } else { "motor" }));
        if p.is_null() {
            self.silencios.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        // `with_mut` falla en loom si otro hilo está dentro a la vez.
        unsafe { &*p }.bloques.with_mut(|b| unsafe { *b += 1 });
        self.motor.store(p, Ordering::Release);
        TRAZA.lock().unwrap().push(format!("{:?} devuelve", loom::thread::current().id()));
        true
    }
}

fn main() {
    let mut modelo = loom::model::Builder::new();
    modelo.preemption_bound = Some(4);
    let ramas = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let cuenta = ramas.clone();
    modelo.check(move || {
        cuenta.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        TRAZA.lock().unwrap().clear();
        let motor = Box::into_raw(Box::new(Motor { bloques: UnsafeCell::new(0) }));
        let ranura = Arc::new(Ranura { motor: AtomicPtr::new(motor), silencios: AtomicUsize::new(0) });
        let hilos: Vec<_> = (0..2)
            .map(|_| {
                let r = ranura.clone();
                thread::spawn(move || (0..2).filter(|_| r.usar()).count())
            })
            .collect();
        // El hilo de control (`with`).
        let control = usize::from(ranura.usar());
        let usados: usize = hilos.into_iter().map(|h| h.join().unwrap()).sum::<usize>() + control;
        let p = ranura.motor.swap(ptr::null_mut(), Ordering::Acquire);
        assert!(!p.is_null(), "el motor se perdió: {:?}", TRAZA.lock().unwrap());
        let motor = unsafe { Box::from_raw(p) };
        let bloques = motor.bloques.with(|b| unsafe { *b });
        assert_eq!(bloques, usados, "bloques calculados y usos no coinciden");
        assert_eq!(usados + ranura.silencios.load(Ordering::Relaxed), 5, "cada intento es un bloque o un silencio");
    });
    println!(
        "loom: {} entrelazados de 3 hilos (2 callbacks de 2 bloques y el control) sin acceso simultáneo, sin perder el puntero y sin devolverlo dos veces",
        ramas.load(std::sync::atomic::Ordering::Relaxed)
    );
}
