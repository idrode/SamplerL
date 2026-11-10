//! Comprobación del propio arnés de loom: el mismo protocolo (tomar y
//! devolver) con un Mutex en vez de atómicos. Si esto «falla», el fallo es
//! del modelo y no del protocolo.
use loom::sync::atomic::{AtomicUsize, Ordering};
use loom::sync::{Arc, Mutex};
use loom::thread;
fn main() {
    let r = std::panic::catch_unwind(|| {
        loom::model(|| {
            let a = Arc::new(Mutex::new(1usize));
            let hilos: Vec<_> = (0..2)
                .map(|_| {
                    let a = a.clone();
                    thread::spawn(move || {
                        let v = std::mem::replace(&mut *a.lock().unwrap(), 0);
                        if v != 0 {
                            *a.lock().unwrap() = v;
                        }
                    })
                })
                .collect();
            for h in hilos {
                h.join().unwrap();
            }
            assert_eq!(*a.lock().unwrap(), 1, "perdido");
        })
    });
    println!("con Mutex: {}", if r.is_ok() { "bien" } else { "FALLA" });
    let r = std::panic::catch_unwind(|| {
        loom::model(|| {
            let a = Arc::new(AtomicUsize::new(1));
            let b = a.clone();
            let h = thread::spawn(move || {
                let v = b.swap(0, Ordering::SeqCst);
                if v != 0 {
                    b.store(v, Ordering::SeqCst);
                }
            });
            h.join().unwrap();
            assert_eq!(a.load(Ordering::SeqCst), 1, "perdido");
        })
    });
    println!("un solo hilo con swap/store SeqCst: {}", if r.is_ok() { "bien" } else { "FALLA" });
    let r = std::panic::catch_unwind(|| {
        loom::model(|| {
            let a = Arc::new(AtomicUsize::new(1));
            let hilos: Vec<_> = (0..2)
                .map(|_| {
                    let b = a.clone();
                    thread::spawn(move || {
                        let v = b.swap(0, Ordering::SeqCst);
                        if v != 0 {
                            b.store(v, Ordering::SeqCst);
                        }
                    })
                })
                .collect();
            for h in hilos {
                h.join().unwrap();
            }
            assert_eq!(a.load(Ordering::SeqCst), 1, "perdido");
        })
    });
    println!("dos hilos con swap/store SeqCst: {}", if r.is_ok() { "bien" } else { "FALLA" });
}
