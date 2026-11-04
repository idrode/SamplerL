//! La rejilla de la ventana principal: del tamaño de la terminal y de lo
//! que hay que enseñar, dónde va cada panel. Es una función pura (sin
//! dibujar ni mirar la aplicación), así se prueba por tamaños y el dibujo
//! y el ratón usan los mismos rectángulos.
//!
//! Cuatro tamaños (ver `docs/DISENO-TUI.md`):
//! - **diminuto** (menos de 40×10): una línea que dice que el audio sigue;
//! - **pequeño** (menos de 100×30): el código a la izquierda y, a la
//!   derecha, la onda en pocas filas y debajo pestañas con lo demás (el
//!   tracker también);
//! - **mediano**: la onda, el tracker y el mezclador lado a lado, y las
//!   pestañas debajo;
//! - **ancho** (desde 160×45): tres columnas (código; onda, tracker y
//!   pestañas; mezclador, scope, espectro y referencia) y los efectos a lo
//!   ancho.
//!
//! Lo que no cabe en su sitio va a las pestañas («dock»): siempre se ve,
//! nunca tapa a otro panel.

use ratatui::layout::Rect;

use crate::focus::Panel;

/// Una pestaña del dock: un panel o una vista que solo vive ahí.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Tab {
    Panel(Panel),
    /// Las visualizaciones del script que no van junto al código.
    Viz,
    Sidechain,
    /// La tabla del último análisis de choques.
    Clash,
    #[default]
    None,
}

impl Tab {
    pub fn label(self, short: bool) -> String {
        match self {
            Tab::Panel(p) if short => format!("{} {}", p.number(), p.short()),
            Tab::Panel(p) => p.title(),
            Tab::Viz => if short { "vis" } else { "visual" }.into(),
            Tab::Sidechain => if short { "sc" } else { "sidechain" }.into(),
            Tab::Clash => if short { "choq" } else { "choques" }.into(),
            Tab::None => String::new(),
        }
    }
}

/// Tamaño de la terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Size {
    Tiny,
    Small,
    Medium,
    Wide,
}

impl Size {
    pub fn of(width: u16, height: u16) -> Size {
        if width < 40 || height < 10 {
            Size::Tiny
        } else if width < 100 || height < 30 {
            Size::Small
        } else if width < 160 || height < 45 {
            Size::Medium
        } else {
            Size::Wide
        }
    }
}

/// Lo que hay que enseñar.
#[derive(Debug, Clone, Default)]
pub struct Needs {
    /// El editor está en esta ventana.
    pub editor: bool,
    /// El piano roll está abierto: va en el sitio del tracker.
    pub roll: bool,
    /// Alto de los efectos a lo ancho (tamaño ancho).
    pub fx_height: u16,
    /// Ancho que pide el mezclador para sus tiras.
    pub mixer_width: u16,
    /// Filas que pide la pestaña que se ve (sin la de las pestañas; 0:
    /// las que haya).
    pub dock_want: u16,
    /// Hay barra de menús.
    pub menu: bool,
    /// Scope y espectro del master.
    pub master_viz: bool,
    /// Las vistas que se abren a mano y van a las pestañas.
    pub occupancy: bool,
    pub viz: bool,
    pub sidechain: bool,
    pub clash: bool,
}

/// El dock: su sitio (con la fila de las pestañas arriba) y sus
/// pestañas, en orden.
#[derive(Debug, Clone, PartialEq)]
pub struct Dock {
    pub area: Rect,
    pub tabs: Vec<Tab>,
}

impl Dock {
    /// El texto de cada pestaña para que quepan en el ancho del dock: los
    /// nombres largos y, si no caben, se acortan los más largos primero.
    pub fn labels(&self) -> Vec<String> {
        let mut short = vec![false; self.tabs.len()];
        let width = |short: &[bool]| -> usize {
            self.tabs
                .iter()
                .zip(short)
                .map(|(t, s)| t.label(*s).chars().count() + 2)
                .sum::<usize>()
                + self.tabs.len().saturating_sub(1)
        };
        let mut order: Vec<usize> = (0..self.tabs.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(self.tabs[i].label(false).chars().count()));
        for i in order {
            if width(&short) <= self.area.width as usize {
                break;
            }
            short[i] = true;
        }
        self.tabs.iter().zip(&short).map(|(t, s)| t.label(*s)).collect()
    }
}

/// Dónde va cada cosa.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    pub size: Size,
    pub menu: Option<Rect>,
    pub header: Option<Rect>,
    /// La regla de pasos y la línea de tiempo.
    pub ruler: Option<Rect>,
    pub editor: Option<Rect>,
    pub wave: Option<Rect>,
    /// El tracker o, abierto, el piano roll (si no está en el dock).
    pub tracker: Option<Rect>,
    pub mixer: Option<Rect>,
    pub fx: Option<Rect>,
    pub lanes: Option<Rect>,
    pub scope: Option<Rect>,
    pub spectrum: Option<Rect>,
    /// La referencia de teclas del foco.
    pub reference: Option<Rect>,
    pub dock: Option<Dock>,
    /// La línea del mensaje.
    pub message: Option<Rect>,
    /// La línea de las teclas del foco (y la de órdenes).
    pub keys: Option<Rect>,
}

impl Grid {
    /// Dónde está `panel` (fuera del dock), si se ve.
    pub fn area_of(&self, panel: Panel) -> Option<Rect> {
        match panel {
            Panel::Code => self.editor,
            Panel::Wave => self.wave,
            Panel::Tracker => self.tracker,
            Panel::Mixer => self.mixer,
            Panel::Fx => self.fx,
            Panel::Lanes => self.lanes,
            Panel::Scope => self.scope,
            Panel::Spectrum => self.spectrum,
            Panel::Occupancy => None,
        }
    }

    /// Si `tab` está en el dock.
    #[cfg(test)]
    pub fn docked(&self, tab: Tab) -> bool {
        self.dock.as_ref().is_some_and(|d| d.tabs.contains(&tab))
    }

    /// Los rectángulos de todo lo que se dibuja (para los tests: ni se
    /// solapan ni se salen).
    #[cfg(test)]
    pub fn rects(&self) -> Vec<Rect> {
        [
            self.menu,
            self.header,
            self.ruler,
            self.editor,
            self.wave,
            self.tracker,
            self.mixer,
            self.fx,
            self.lanes,
            self.scope,
            self.spectrum,
            self.reference,
            self.dock.as_ref().map(|d| d.area),
            self.message,
            self.keys,
        ]
        .into_iter()
        .flatten()
        .filter(|r| r.width > 0 && r.height > 0)
        .collect()
    }
}

/// Alto del tracker con sus 16 filas (cabecera y bordes).
pub const TRACKER_FULL: u16 = engine::STEPS as u16 + 3;
/// Lo mínimo para que un panel con borde diga algo.
const MIN_PANEL: u16 = 3;
/// Ancho de la columna derecha del tamaño ancho.
const WIDE_RIGHT: u16 = 50;

/// Corta `n` filas de arriba de `area`: (las de arriba, lo demás).
fn take_top(area: Rect, n: u16) -> (Rect, Rect) {
    let n = n.min(area.height);
    (
        Rect { height: n, ..area },
        Rect {
            y: area.y + n,
            height: area.height - n,
            ..area
        },
    )
}

/// Lo mismo por abajo: (lo demás, las de abajo).
fn take_bottom(area: Rect, n: u16) -> (Rect, Rect) {
    let n = n.min(area.height);
    (
        Rect {
            height: area.height - n,
            ..area
        },
        Rect {
            y: area.y + area.height - n,
            height: n,
            ..area
        },
    )
}

/// Corta `n` columnas por la izquierda: (las de la izquierda, lo demás).
fn take_left(area: Rect, n: u16) -> (Rect, Rect) {
    let n = n.min(area.width);
    (
        Rect { width: n, ..area },
        Rect {
            x: area.x + n,
            width: area.width - n,
            ..area
        },
    )
}

/// Lo mismo por la derecha: (lo demás, las de la derecha).
fn take_right(area: Rect, n: u16) -> (Rect, Rect) {
    let n = n.min(area.width);
    (
        Rect {
            width: area.width - n,
            ..area
        },
        Rect {
            x: area.x + area.width - n,
            width: n,
            ..area
        },
    )
}

fn some(area: Rect) -> Option<Rect> {
    (area.width > 0 && area.height > 0).then_some(area)
}

/// Reparte el alto de `area` entre la onda (hasta `wave_max`), el
/// tracker (sus 16 filas si caben) y el dock (al menos `dock_min`, o lo
/// que pida su pestaña más la fila de las pestañas): para darle al dock lo
/// que pide se aprieta antes la onda (hasta 6) y luego el tracker (hasta
/// 7, y se desplaza). Lo que sobre, al dock.
fn stack(area: Rect, wave_max: u16, dock_min: u16, dock_want: u16) -> (Rect, Rect, Rect) {
    let h = area.height;
    let wave_floor = 6.min(h / 3);
    let tracker_floor = (MIN_PANEL + 4).min(h - wave_floor);
    let wanted = if dock_want > 0 { dock_want + 1 } else { 0 };
    let dock = dock_min
        .max(wanted)
        .min(h.saturating_sub(wave_floor + tracker_floor))
        .max(dock_min.min(h.saturating_sub(wave_floor + tracker_floor)));
    let rest = h - dock;
    let wave = rest
        .saturating_sub(TRACKER_FULL)
        .clamp(wave_floor, wave_max.max(wave_floor))
        .min(rest);
    let tracker = (rest - wave).min(TRACKER_FULL);
    let (wave, rest) = take_top(area, wave);
    let (seq, dock) = take_top(rest, tracker);
    (wave, seq, dock)
}

/// Las pestañas de las vistas que se abren a mano, en su orden.
fn extra_tabs(needs: &Needs, tabs: &mut Vec<Tab>) {
    if needs.occupancy {
        tabs.push(Tab::Panel(Panel::Occupancy));
    }
    if needs.viz {
        tabs.push(Tab::Viz);
    }
    if needs.sidechain {
        tabs.push(Tab::Sidechain);
    }
    if needs.clash {
        tabs.push(Tab::Clash);
    }
}

fn dock(area: Rect, tabs: Vec<Tab>) -> Option<Dock> {
    let area = some(area)?;
    Some(Dock { area, tabs })
}

/// La rejilla para una terminal de `screen` con `needs`.
pub fn grid(screen: Rect, needs: &Needs) -> Grid {
    let size = Size::of(screen.width, screen.height);
    let mut g = Grid {
        size,
        menu: None,
        header: None,
        ruler: None,
        editor: None,
        wave: None,
        tracker: None,
        mixer: None,
        fx: None,
        lanes: None,
        scope: None,
        spectrum: None,
        reference: None,
        dock: None,
        message: None,
        keys: None,
    };
    if size == Size::Tiny {
        // Solo el transporte y el mensaje.
        let (header, rest) = take_top(screen, 1);
        g.header = some(header);
        g.message = some(rest);
        return g;
    }
    let mut rest = screen;
    // Arriba: menús (con sitio), cabecera y regla.
    if needs.menu && size >= Size::Medium {
        let (menu, r) = take_top(rest, 1);
        g.menu = some(menu);
        rest = r;
    }
    let (header, r) = take_top(rest, 1);
    g.header = some(header);
    rest = r;
    if screen.height >= 20 {
        let (ruler, r) = take_top(rest, 1);
        g.ruler = some(ruler);
        rest = r;
    }
    // Abajo: las teclas y el mensaje.
    let (r, keys) = take_bottom(rest, 1);
    g.keys = some(keys);
    rest = r;
    if screen.height >= 16 {
        let (r, message) = take_bottom(rest, 1);
        g.message = some(message);
        rest = r;
    }
    match size {
        Size::Tiny => unreachable!(),
        Size::Small => small(&mut g, rest, needs),
        Size::Medium => medium(&mut g, rest, needs),
        Size::Wide => wide(&mut g, rest, needs),
    }
    g
}

/// Código a la izquierda; a la derecha, la onda en pocas filas y las
/// pestañas con todo lo demás.
fn small(g: &mut Grid, body: Rect, needs: &Needs) {
    let right = if needs.editor {
        let (editor, right) = take_left(body, body.width / 2);
        g.editor = some(editor);
        right
    } else {
        body
    };
    // La onda: bordes y dos filas (una si no hay alto).
    let wave = if right.height >= 14 { 4 } else { MIN_PANEL };
    let (wave, rest) = take_top(right, wave);
    g.wave = some(wave);
    let mut tabs = vec![
        Tab::Panel(Panel::Tracker),
        Tab::Panel(Panel::Mixer),
        Tab::Panel(Panel::Fx),
        Tab::Panel(Panel::Lanes),
    ];
    if needs.master_viz {
        tabs.push(Tab::Panel(Panel::Scope));
        tabs.push(Tab::Panel(Panel::Spectrum));
    }
    extra_tabs(needs, &mut tabs);
    g.dock = dock(rest, tabs);
}

/// Código a la izquierda; a la derecha, la onda, el tracker con el
/// mezclador al lado y las pestañas debajo.
fn medium(g: &mut Grid, body: Rect, needs: &Needs) {
    let right = if needs.editor {
        let (editor, right) = take_left(body, body.width / 2);
        g.editor = some(editor);
        right
    } else {
        body
    };
    let wave = if needs.editor { 8 } else { right.height / 2 };
    let (wave, seq, dock_area) = stack(right, wave, 6, needs.dock_want);
    g.wave = some(wave);
    let mut tabs = vec![Tab::Panel(Panel::Fx), Tab::Panel(Panel::Lanes)];
    // El mezclador al lado del tracker; con el piano roll, a las pestañas
    // (el roll necesita el ancho).
    if needs.roll {
        g.tracker = some(seq);
        tabs.insert(0, Tab::Panel(Panel::Mixer));
    } else {
        let mixer = needs.mixer_width.min(seq.width / 2);
        let (tracker, mixer) = take_right(seq, mixer);
        g.tracker = some(tracker);
        g.mixer = some(mixer);
    }
    if needs.master_viz {
        tabs.push(Tab::Panel(Panel::Scope));
        tabs.push(Tab::Panel(Panel::Spectrum));
    }
    extra_tabs(needs, &mut tabs);
    g.dock = dock(dock_area, tabs);
}

/// Tres columnas y los efectos a lo ancho.
fn wide(g: &mut Grid, body: Rect, needs: &Needs) {
    let (body, fx) = take_bottom(body, needs.fx_height.clamp(MIN_PANEL, body.height / 4));
    g.fx = some(fx);
    let (rest, right) = take_right(body, WIDE_RIGHT);
    let centre = if needs.editor {
        let (editor, centre) = take_left(rest, rest.width * 9 / 20);
        g.editor = some(editor);
        centre
    } else {
        rest
    };
    // Derecha: el mezclador arriba, luego scope y espectro, y la
    // referencia con lo que quede.
    let mixer = (right.height / 2).clamp(MIN_PANEL, 16);
    let (mixer, r) = take_top(right, mixer);
    g.mixer = some(mixer);
    let r = if needs.master_viz {
        let (scope, r) = take_top(r, 7.min(r.height / 2));
        let (spectrum, r) = take_top(r, 6.min(r.height / 2));
        g.scope = some(scope);
        g.spectrum = some(spectrum);
        r
    } else {
        r
    };
    g.reference = some(r);
    // Centro: la onda, el tracker entero si cabe y las lanes o las
    // pestañas abajo.
    let wave = if needs.editor { 10 } else { 14 };
    let (wave, seq, dock_area) = stack(centre, wave, 8, needs.dock_want);
    g.wave = some(wave);
    g.tracker = some(seq);
    let mut tabs = vec![Tab::Panel(Panel::Lanes)];
    extra_tabs(needs, &mut tabs);
    g.dock = dock(dock_area, tabs);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn needs() -> Needs {
        Needs {
            editor: true,
            mixer_width: 26,
            fx_height: 9,
            menu: true,
            master_viz: true,
            ..Default::default()
        }
    }

    /// Nada se solapa ni se sale de la pantalla.
    fn check(screen: Rect, g: &Grid) {
        let rects = g.rects();
        for (i, a) in rects.iter().enumerate() {
            assert!(
                screen.contains(a.as_position()) && a.right() <= screen.right() && a.bottom() <= screen.bottom(),
                "{a:?} se sale de {screen:?}"
            );
            for b in &rects[i + 1..] {
                assert!(!a.intersects(*b), "{a:?} y {b:?} se solapan en {screen:?}");
            }
        }
    }

    #[test]
    fn no_size_overlaps_or_spills() {
        for (w, h) in [
            (20, 5),
            (39, 9),
            (40, 10),
            (60, 16),
            (80, 24),
            (99, 29),
            (100, 30),
            (120, 40),
            (159, 44),
            (160, 45),
            (180, 50),
            (250, 70),
        ] {
            let screen = Rect::new(0, 0, w, h);
            for editor in [true, false] {
                for roll in [true, false] {
                    let n = Needs {
                        editor,
                        roll,
                        occupancy: true,
                        sidechain: true,
                        ..needs()
                    };
                    check(screen, &grid(screen, &n));
                }
            }
        }
    }

    #[test]
    fn the_longest_tab_names_shorten_first() {
        let dock = |width| Dock {
            area: Rect::new(0, 0, width, 5),
            tabs: vec![
                Tab::Panel(Panel::Tracker),
                Tab::Panel(Panel::Mixer),
                Tab::Panel(Panel::Fx),
                Tab::Panel(Panel::Lanes),
            ],
        };
        assert_eq!(dock(60).labels(), ["3 tracker", "4 mezcla", "5 efectos", "6 lanes"]);
        // A 40 basta con acortar el más largo.
        assert_eq!(dock(40).labels(), ["3 trk", "4 mezcla", "5 efectos", "6 lanes"]);
        assert_eq!(dock(36).labels(), ["3 trk", "4 mezcla", "5 fx", "6 lanes"]);
        assert_eq!(dock(20).labels(), ["3 trk", "4 mez", "5 fx", "6 lan"]);
    }

    #[test]
    fn the_dock_grows_to_what_its_tab_asks_for() {
        let screen = Rect::new(0, 0, 120, 40);
        let plain = grid(screen, &needs());
        let tall = grid(
            screen,
            &Needs {
                dock_want: 14,
                ..needs()
            },
        );
        let (a, b) = (plain.dock.unwrap().area, tall.dock.unwrap().area);
        assert!(b.height >= 15 && b.height > a.height, "{a:?} {b:?}");
        // Para eso se aprieta el tracker (la onda ya estaba en lo mínimo).
        assert!(tall.tracker.unwrap().height < TRACKER_FULL);
        assert!(tall.wave.unwrap().height >= 6);
    }

    #[test]
    fn sizes() {
        assert_eq!(Size::of(39, 24), Size::Tiny);
        assert_eq!(Size::of(80, 9), Size::Tiny);
        assert_eq!(Size::of(80, 24), Size::Small);
        assert_eq!(Size::of(120, 40), Size::Medium);
        assert_eq!(Size::of(180, 50), Size::Wide);
        assert_eq!(Size::of(180, 40), Size::Medium);
    }

    #[test]
    fn tiny_is_only_the_transport_and_a_message() {
        let g = grid(Rect::new(0, 0, 30, 8), &needs());
        assert!(g.header.is_some() && g.message.is_some());
        assert!(g.editor.is_none() && g.wave.is_none() && g.dock.is_none());
    }

    #[test]
    fn small_puts_the_tracker_and_the_mixer_in_tabs() {
        let g = grid(Rect::new(0, 0, 80, 24), &needs());
        assert_eq!(g.size, Size::Small);
        assert!(g.menu.is_none(), "a 80 columnas la barra de menús no ocupa fila");
        assert_eq!(g.editor.unwrap().width, 40);
        assert!(g.wave.is_some());
        assert!(g.tracker.is_none() && g.mixer.is_none());
        let dock = g.dock.as_ref().unwrap();
        assert_eq!(dock.tabs[0], Tab::Panel(Panel::Tracker));
        assert!(dock.tabs.contains(&Tab::Panel(Panel::Mixer)));
        assert!(dock.tabs.contains(&Tab::Panel(Panel::Fx)));
        // Cabecera, regla, cuerpo, mensaje y teclas: 24 filas.
        assert_eq!(g.header.unwrap().y, 0);
        assert_eq!(g.ruler.unwrap().y, 1);
        assert_eq!(g.keys.unwrap().y, 23);
        assert_eq!(g.message.unwrap().y, 22);
    }

    #[test]
    fn medium_has_the_full_tracker_beside_the_mixer() {
        let g = grid(Rect::new(0, 0, 120, 40), &needs());
        assert_eq!(g.size, Size::Medium);
        assert!(g.menu.is_some());
        assert_eq!(g.tracker.unwrap().height, TRACKER_FULL);
        let (tracker, mixer) = (g.tracker.unwrap(), g.mixer.unwrap());
        assert_eq!(tracker.right(), mixer.x);
        assert_eq!(mixer.width, 26);
        let dock = g.dock.as_ref().unwrap();
        assert_eq!(dock.tabs[0], Tab::Panel(Panel::Fx));
        assert!(dock.area.height >= 6);
        // Con el piano roll, el mezclador pasa a las pestañas.
        let g = grid(
            Rect::new(0, 0, 120, 40),
            &Needs {
                roll: true,
                ..needs()
            },
        );
        assert!(g.mixer.is_none());
        assert!(g.docked(Tab::Panel(Panel::Mixer)));
    }

    #[test]
    fn wide_has_three_columns_and_the_effects_below() {
        let g = grid(Rect::new(0, 0, 180, 50), &needs());
        assert_eq!(g.size, Size::Wide);
        let (editor, wave, mixer) = (g.editor.unwrap(), g.wave.unwrap(), g.mixer.unwrap());
        assert!(editor.right() <= wave.x && wave.right() <= mixer.x);
        assert_eq!(mixer.width, WIDE_RIGHT);
        assert!(g.scope.is_some() && g.spectrum.is_some() && g.reference.is_some());
        assert_eq!(g.fx.unwrap().width, 180);
        assert_eq!(g.tracker.unwrap().height, TRACKER_FULL);
        assert!(g.docked(Tab::Panel(Panel::Lanes)));
    }

    #[test]
    fn without_the_editor_the_sound_takes_its_place() {
        let n = Needs {
            editor: false,
            ..needs()
        };
        for (w, h) in [(80, 24), (120, 40), (180, 50)] {
            let g = grid(Rect::new(0, 0, w, h), &n);
            assert!(g.editor.is_none());
            assert_eq!(g.wave.unwrap().x, 0, "{w}×{h}");
        }
    }
}
