//! Dibujar sin que un fallo del dibujo pare el programa (y con él el
//! audio): un pánico dentro de un widget se recoge y ese cuadro sale con
//! una pantalla mínima; un error de la terminal al escribir se salta el
//! cuadro. El bucle sigue, la entrada sigue y el motor sigue sonando.
//!
//! El aviso de un pánico no se escribe en la terminal (rompería la
//! pantalla): el gancho de pánicos lo guarda mientras se dibuja y lo
//! devuelve aquí.

use std::cell::{Cell, RefCell};
use std::panic::{self, AssertUnwindSafe};
use std::sync::Once;

use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};

thread_local! {
    /// Si este hilo está dibujando (el gancho calla y guarda el mensaje).
    static DRAWING: Cell<bool> = const { Cell::new(false) };
    static LAST_PANIC: RefCell<Option<String>> = const { RefCell::new(None) };
}

static HOOK: Once = Once::new();

/// Pone (una vez) el gancho de pánicos que, mientras se dibuja, guarda el
/// mensaje en vez de escribirlo; fuera del dibujo, el de siempre.
fn install_hook() {
    HOOK.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if DRAWING.with(Cell::get) {
                let place = info
                    .location()
                    .map_or(String::new(), |l| format!(" ({}:{})", l.file(), l.line()));
                let text = info
                    .payload()
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| info.payload().downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "pánico".into());
                LAST_PANIC.with(|p| *p.borrow_mut() = Some(format!("{text}{place}")));
            } else {
                previous(info);
            }
        }));
    });
}

/// Lo que pasó al dibujar un cuadro.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Drawn,
    /// Un widget entró en pánico: el cuadro salió con la pantalla mínima.
    Panicked(String),
    /// La terminal no dejó escribir: el cuadro no salió.
    Failed(String),
}

/// Dibuja un cuadro con `draw`. Nunca entra en pánico ni devuelve error:
/// lo que falle queda en el [`Outcome`].
pub fn draw<B: Backend>(terminal: &mut Terminal<B>, draw: impl FnOnce(&mut Frame)) -> Outcome {
    install_hook();
    let mut panicked = None;
    let result = terminal.draw(|frame| {
        DRAWING.with(|d| d.set(true));
        let caught = panic::catch_unwind(AssertUnwindSafe(|| draw(frame)));
        DRAWING.with(|d| d.set(false));
        if caught.is_err() {
            let message = LAST_PANIC
                .with(|p| p.borrow_mut().take())
                .unwrap_or_else(|| "pánico".into());
            fallback(frame, &message);
            panicked = Some(message);
        }
    });
    match (result, panicked) {
        (Err(e), _) => Outcome::Failed(e.to_string()),
        // Lo que el widget dejó a medias se borró antes de escribir y el
        // gancho no escribe nada: la terminal tiene la pantalla mínima y el
        // siguiente cuadro sale bien con la diferencia de siempre.
        (Ok(_), Some(message)) => Outcome::Panicked(message),
        (Ok(_), None) => Outcome::Drawn,
    }
}

/// La pantalla mínima: qué pasó y que el audio sigue.
pub fn fallback(frame: &mut Frame, message: &str) {
    let area = frame.area();
    frame.buffer_mut().reset();
    let lines = vec![
        Line::from("el dibujo ha fallado en este cuadro · el audio sigue"),
        Line::from(message.to_string()),
        Line::from("Ctrl-Q salir · F12 pánico · Ctrl-Espacio marcha/paro"),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    #[test]
    fn a_panic_while_drawing_is_caught_and_the_next_frame_draws() {
        let mut terminal = Terminal::new(TestBackend::new(60, 6)).unwrap();
        let outcome = draw(&mut terminal, |_| panic!("roto"));
        let Outcome::Panicked(message) = outcome else {
            panic!("se esperaba un pánico recogido: {outcome:?}");
        };
        assert!(message.starts_with("roto"), "{message}");
        let text = format!("{:?}", terminal.backend().buffer());
        assert!(text.contains("el audio sigue"), "{text}");
        // Y el siguiente cuadro sale normal.
        let outcome = draw(&mut terminal, |f| {
            f.render_widget(Line::from("bien"), f.area());
        });
        assert_eq!(outcome, Outcome::Drawn);
        let text = format!("{:?}", terminal.backend().buffer());
        assert!(text.contains("bien") && !text.contains("audio sigue"), "{text}");
    }
}
