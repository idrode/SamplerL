//! Red contra los pánicos fuera del dibujo: la carga del script, los
//! proyectos, los samples y los hilos auxiliares de la interfaz.
//!
//! Un pánico es un fallo del programa, no del script, pero no debe matar el
//! proceso (y con él el audio): [`catch`] lo recoge y lo devuelve como
//! texto, la carga falla con ese error en el editor y sigue sonando lo
//! anterior (regla 5). Mientras se recoge, el aviso no se escribe en la
//! terminal (rompería la pantalla de la interfaz): el gancho de pánicos lo
//! guarda y [`catch`] lo devuelve. Fuera de [`catch`], el gancho de
//! siempre.
//!
//! Con [`catch_at`], el gancho pregunta además dónde iba el script en el
//! momento del pánico (el gancho corre antes de deshacer la pila, así que
//! la de Lua sigue entera).

use std::cell::{Cell, RefCell};
use std::panic::{self, AssertUnwindSafe};
use std::sync::Once;

/// Dónde iba el código del usuario («song.lua:12: »), si se sabe.
type Place = Box<dyn Fn() -> String>;

thread_local! {
    /// Cuántos [`catch`] hay abiertos en este hilo (el gancho calla si hay
    /// alguno).
    static DEPTH: Cell<u32> = const { Cell::new(0) };
    static LAST: RefCell<Option<String>> = const { RefCell::new(None) };
    static PLACE: RefCell<Option<Place>> = const { RefCell::new(None) };
}

static HOOK: Once = Once::new();

/// Pone (una vez) el gancho que, dentro de [`catch`], guarda el mensaje en
/// vez de escribirlo; fuera, llama al que hubiera.
fn install_hook() {
    HOOK.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if DEPTH.with(Cell::get) == 0 {
                previous(info);
                return;
            }
            let text = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "pánico".into());
            let code = info
                .location()
                .map_or(String::new(), |l| format!(" ({}:{})", l.file(), l.line()));
            // Solo el primero: un segundo pánico al deshacer no lo tapa.
            LAST.with(|last| {
                let mut last = last.borrow_mut();
                if last.is_none() {
                    let place = PLACE.with(|p| p.borrow().as_ref().map(|f| f()));
                    *last = Some(format!("{}{text}{code}", place.unwrap_or_default()));
                }
            });
        }));
    });
}

/// Ejecuta `f`; si entra en pánico, devuelve el mensaje del pánico (con el
/// archivo y la línea del código de Rust) en vez de seguir deshaciendo la
/// pila. No escribe nada en la terminal.
pub fn catch<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    install_hook();
    DEPTH.with(|d| d.set(d.get() + 1));
    let result = panic::catch_unwind(AssertUnwindSafe(f));
    DEPTH.with(|d| d.set(d.get() - 1));
    // El mensaje es del pánico más interno; uno ya recogido por un
    // `catch` de dentro no cuenta.
    let message = LAST.with(|last| last.borrow_mut().take());
    result.map_err(|_| message.unwrap_or_else(|| "pánico".into()))
}

/// Como [`catch`], y el mensaje empieza por lo que diga `place` en el
/// momento del pánico (dónde iba el script).
pub fn catch_at<T>(
    place: impl Fn() -> String + 'static,
    f: impl FnOnce() -> T,
) -> Result<T, String> {
    let previous = PLACE.with(|p| p.borrow_mut().replace(Box::new(place)));
    let result = catch(f);
    PLACE.with(|p| *p.borrow_mut() = previous);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_becomes_its_message_and_the_next_call_works() {
        let error = catch(|| -> u32 { panic!("roto {}", 3) }).unwrap_err();
        assert!(error.starts_with("roto 3 ("), "{error}");
        assert!(error.contains("panics.rs"), "{error}");
        assert_eq!(catch(|| 7), Ok(7));
    }

    #[test]
    fn the_place_goes_first_and_only_inside() {
        let error = catch_at(|| "song.lua:4: ".into(), || panic!("x")).unwrap_err();
        assert!(error.starts_with("song.lua:4: x"), "{error}");
        let error = catch(|| panic!("y")).unwrap_err();
        assert!(error.starts_with("y ("), "{error}");
    }

    #[test]
    fn nested_catches_each_get_their_own_panic() {
        let outer = catch(|| {
            let inner = catch(|| panic!("dentro")).unwrap_err();
            assert!(inner.starts_with("dentro"));
            panic!("fuera")
        })
        .unwrap_err();
        assert!(outer.starts_with("fuera"), "{outer}");
    }
}
