//! Dónde va cada cosa que se puede mover a mano: el ancho del explorador,
//! el alto del editor de abajo, el editor maximizado y el flotante. Se
//! cambia con el ratón (arrastrando un borde o el título) o con las teclas
//! (Ctrl-Alt-flechas, Alt-Z...). El alto del editor y el ancho del
//! explorador se guardan en `[interfaz]` de `config.toml`, sin tocar lo
//! demás, para la próxima vez.

use ratatui::layout::Rect;

use super::{App, MouseAction};
use crate::layout::{self, EditorPlace};

/// Lo que se arrastra con el ratón.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EdgeDrag {
    /// El borde de arriba del editor acoplado (`moved`: se ha movido; si
    /// no, al soltar es un clic que da el foco).
    EditorTop { moved: bool },
    /// El título del flotante: lo mueve (dónde se agarró, dentro de él).
    FloatMove { dx: u16, dy: u16 },
    /// El borde de abajo o el de la derecha del flotante: su tamaño.
    FloatResize { right: bool, bottom: bool },
}

/// Lo que se mueve el flotante con cada Alt-Shift-flecha.
const FLOAT_STEP: (i32, i32) = (2, 1);

impl App {
    /// Guarda en la configuración lo que se ha movido (si hay archivo de
    /// configuración: en los tests, no).
    pub(super) fn persist_layout(&mut self) {
        let Some(path) = self.devices_config.clone() else {
            return;
        };
        let values = [
            ("explorador", self.files.shown.to_string()),
            ("ancho_explorador", self.files.width.to_string()),
            ("alto_editor", self.editor_rows.to_string()),
        ];
        if let Err(e) = crate::config::save_keys(&path, "interfaz", &values) {
            self.message = format!("no se pudo guardar en {}: {e}", path.display());
        }
    }

    /// Dónde va el editor y su alto acoplado (para la rejilla).
    pub(super) fn editor_layout(&self) -> (EditorPlace, u16) {
        (self.editor_place, self.editor_rows)
    }

    /// El alto del cuerpo de la ventana en el último dibujo (sin él, uno
    /// de 40 filas).
    fn body(&self) -> Rect {
        self.grid
            .as_ref()
            .and_then(|g| g.body)
            .unwrap_or(Rect::new(0, 0, 120, 35))
    }

    /// El alto que tiene ahora el editor acoplado (el pedido, si se ha
    /// pedido uno: entre dos dibujos la rejilla aún no lo sabe).
    fn docked_rows(&self) -> u16 {
        if self.editor_rows > 0 {
            return layout::editor_rows(self.body().height, self.editor_rows);
        }
        match self.grid.as_ref() {
            Some(g) if g.editor_place == EditorPlace::Docked && g.editor.is_some() => {
                g.editor.map_or(0, |r| r.height)
            }
            _ => layout::editor_rows(self.body().height, self.editor_rows),
        }
    }

    /// Pone el editor acoplado a `rows` filas (dentro de sus límites).
    fn set_editor_rows(&mut self, rows: u16) {
        self.editor_rows = layout::editor_rows(self.body().height, rows.max(1));
    }

    /// Ctrl-Alt-↑↓: una fila más o menos para el editor.
    pub(super) fn resize_editor(&mut self, delta: i32) {
        match self.editor_place {
            EditorPlace::Docked => {
                let rows = (self.docked_rows() as i32 + delta).max(1) as u16;
                self.set_editor_rows(rows);
                self.message = format!("editor: {} filas", self.editor_rows);
                self.persist_layout();
            }
            EditorPlace::Maximized => {
                self.message = "el editor está maximizado: Alt-Z lo devuelve a su sitio".into();
            }
            EditorPlace::Floating(_) => self.resize_float(0, delta),
        }
    }

    /// Ctrl-Alt-←→: el flotante, si flota; si no, el explorador.
    pub(super) fn widen(&mut self, wider: bool) {
        let step = super::files::WIDTH_STEP as i32;
        let delta = if wider { step } else { -step };
        match self.editor_place {
            EditorPlace::Floating(_) => self.resize_float(delta, 0),
            _ => self.resize_explorer(delta),
        }
    }

    /// Alt-Z: el editor ocupa todo el centro, o vuelve a donde estaba.
    pub(super) fn toggle_maximized(&mut self) {
        if self.is_main_window() {
            self.message = "el editor está en su ventana".into();
            return;
        }
        if self.editor_place == EditorPlace::Maximized {
            self.editor_place = self.editor_restore;
            self.message = "editor en su sitio".into();
        } else {
            self.editor_restore = self.editor_place;
            self.editor_place = EditorPlace::Maximized;
            self.message = "editor maximizado (el audio sigue): Alt-Z lo devuelve".into();
        }
        self.completion = None;
    }

    /// El ratón en los bordes que se arrastran: `true` si era suyo. El de
    /// arriba del editor acoplado cambia su alto; un clic sin moverlo es
    /// un clic en el editor (le da el foco).
    pub(super) fn dock_mouse(&mut self, action: MouseAction, x: u16, y: u16) -> bool {
        if let Some(drag) = self.edge_drag {
            match (action, drag) {
                (MouseAction::Drag, EdgeDrag::FloatMove { .. } | EdgeDrag::FloatResize { .. }) => {
                    self.float_drag(drag, x, y)
                }
                (MouseAction::Up, EdgeDrag::FloatMove { .. } | EdgeDrag::FloatResize { .. }) => {
                    self.edge_drag = None;
                    if let Some(r) = self.float_now() {
                        self.message = format!(
                            "editor flotante: {}×{} en ({}, {})",
                            r.width, r.height, r.x, r.y
                        );
                    }
                }
                (MouseAction::Drag, EdgeDrag::EditorTop { .. }) => {
                    if let Some(editor) = self.grid.as_ref().and_then(|g| g.editor) {
                        let rows = editor.bottom().saturating_sub(y);
                        self.set_editor_rows(rows);
                    }
                    self.edge_drag = Some(EdgeDrag::EditorTop { moved: true });
                }
                (MouseAction::Up, EdgeDrag::EditorTop { moved }) => {
                    self.edge_drag = None;
                    if moved {
                        self.message = format!("editor: {} filas", self.editor_rows);
                        self.persist_layout();
                    } else {
                        self.set_focus(crate::focus::Panel::Code);
                    }
                }
                _ => self.edge_drag = None,
            }
            return true;
        }
        if action != MouseAction::Down {
            return false;
        }
        if self.float_mouse_down(x, y) {
            return true;
        }
        let Some(grid) = self.grid.as_ref() else {
            return false;
        };
        let Some(editor) = grid.editor else {
            return false;
        };
        let at = ratatui::layout::Position::new(x, y);
        if grid.editor_place == EditorPlace::Docked
            && y == editor.y
            && editor.contains(at)
            && !grid.covered(crate::focus::Panel::Code, x, y)
        {
            self.edge_drag = Some(EdgeDrag::EditorTop { moved: false });
            return true;
        }
        false
    }

    /// El flotante como está ahora (el pedido, o el del último dibujo).
    fn float_now(&self) -> Option<Rect> {
        let EditorPlace::Floating(wanted) = self.editor_place else {
            return None;
        };
        // El pedido primero: entre dos dibujos la rejilla aún no lo sabe.
        Some(
            wanted
                .or_else(|| {
                    self.grid
                        .as_ref()
                        .filter(|g| g.floating)
                        .and_then(|g| g.editor)
                })
                .unwrap_or_else(|| layout::float_rect(self.body(), None)),
        )
    }

    /// Pone el flotante en `r` (dentro del cuerpo y no más pequeño que su
    /// mínimo).
    fn set_float(&mut self, r: Rect) {
        let r = layout::float_rect(self.body(), Some(r));
        self.editor_place = EditorPlace::Floating(Some(r));
        self.last_float = Some(r);
    }

    /// Cambia el tamaño del flotante (columnas y filas).
    pub(super) fn resize_float(&mut self, columns: i32, rows: i32) {
        let Some(r) = self.float_now() else {
            return;
        };
        let grow = |n: u16, d: i32| (n as i32 + d).max(1) as u16;
        self.set_float(Rect {
            width: grow(r.width, columns),
            height: grow(r.height, rows),
            ..r
        });
    }

    /// Alt-Shift-flechas: mueve el flotante.
    pub(super) fn move_float(&mut self, columns: i32, rows: i32) {
        let Some(r) = self.float_now() else {
            self.message = "el editor no flota: Alt-W lo saca encima".into();
            return;
        };
        let shift = |n: u16, d: i32| (n as i32 + d).max(0) as u16;
        self.set_float(Rect {
            x: shift(r.x, columns * FLOAT_STEP.0),
            y: shift(r.y, rows * FLOAT_STEP.1),
            ..r
        });
    }

    /// Alt-W: el editor flota encima de todo (donde se dejó la última vez)
    /// o vuelve abajo.
    pub(super) fn toggle_float(&mut self) {
        if self.is_main_window() {
            self.message = "el editor está en su ventana".into();
            return;
        }
        self.completion = None;
        self.editor_place = match self.editor_place {
            EditorPlace::Floating(_) => {
                self.message = "editor abajo".into();
                EditorPlace::Docked
            }
            _ => {
                self.message =
                    "editor flotante: arrastra su título para moverlo y sus bordes para el tamaño · Alt-Shift-flechas lo mueven · Ctrl-Alt-flechas, su tamaño · Alt-W lo devuelve".into();
                EditorPlace::Floating(self.last_float)
            }
        };
        if self.mode != super::Mode::Edit && self.chosen_focus.is_none() {
            self.set_focus(crate::focus::Panel::Code);
        }
    }

    /// El ratón sobre el flotante: el título lo mueve, los bordes de abajo
    /// y de la derecha cambian su tamaño (`true`: era suyo).
    fn float_mouse_down(&mut self, x: u16, y: u16) -> bool {
        let Some(grid) = self.grid.as_ref().filter(|g| g.floating) else {
            return false;
        };
        let Some(r) = grid.editor else {
            return false;
        };
        if !r.contains(ratatui::layout::Position::new(x, y)) {
            return false;
        }
        let right = x + 1 == r.right();
        let bottom = y + 1 == r.bottom();
        if right || bottom {
            self.edge_drag = Some(EdgeDrag::FloatResize { right, bottom });
            return true;
        }
        if y == r.y {
            self.edge_drag = Some(EdgeDrag::FloatMove { dx: x - r.x, dy: 0 });
            return true;
        }
        false
    }

    /// Arrastrando el flotante.
    fn float_drag(&mut self, drag: EdgeDrag, x: u16, y: u16) {
        let Some(r) = self.float_now() else {
            return;
        };
        match drag {
            EdgeDrag::FloatMove { dx, dy } => self.set_float(Rect {
                x: x.saturating_sub(dx),
                y: y.saturating_sub(dy),
                ..r
            }),
            EdgeDrag::FloatResize { right, bottom } => self.set_float(Rect {
                width: if right {
                    (x + 1).saturating_sub(r.x)
                } else {
                    r.width
                },
                height: if bottom {
                    (y + 1).saturating_sub(r.y)
                } else {
                    r.height
                },
                ..r
            }),
            EdgeDrag::EditorTop { .. } => {}
        }
    }
}
