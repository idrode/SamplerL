//! La rejilla de la ventana principal: del tamaño de la terminal y de lo
//! que hay que enseñar, dónde va cada panel. Es una función pura (sin
//! dibujar ni mirar la aplicación), así se prueba por tamaños y el dibujo
//! y el ratón usan los mismos rectángulos.
//!
//! La disposición (ver «Explorador y editor abajo» en `docs/DISENO-TUI.md`):
//! arriba los menús, la cabecera y la regla; a la izquierda el explorador
//! de archivos; en el centro, arriba los paneles del sonido y abajo el
//! editor a todo el ancho del centro (con su alto, maximizado o flotando
//! encima); abajo del todo el mensaje y las teclas.
//!
//! Los paneles del sonido, por tamaños:
//! - **diminuto** (menos de 40×10): una línea que dice que el audio sigue;
//! - **pequeño** (menos de 100×30): la onda en pocas filas y debajo
//!   pestañas con lo demás (el tracker también);
//! - **mediano**: la onda, el tracker y el mezclador lado a lado, y debajo
//!   las pestañas con el espectro del master al lado;
//! - **ancho** (desde 160×45): la onda, el tracker y las pestañas, y una
//!   columna con el mezclador, el scope y el espectro del master.
//!
//! Lo que no cabe en su sitio va a las pestañas («dock»): siempre se ve,
//! nunca tapa a otro panel. Solo tapan el editor flotante y el explorador
//! abierto a mano en una terminal estrecha: los dos se apuntan en la
//! rejilla y el ratón los mira antes que lo de debajo.

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

/// Dónde va el editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditorPlace {
    /// Abajo, a todo el ancho del centro, con su alto.
    #[default]
    Docked,
    /// Todo el centro (los paneles del sonido no se ven).
    Maximized,
    /// Encima de todo, donde se dejó (`None`: en el centro).
    Floating(Option<Rect>),
}

/// Lo que hay que enseñar.
#[derive(Debug, Clone, Default)]
pub struct Needs {
    /// El editor está en esta ventana.
    pub editor: bool,
    /// Dónde, y su alto acoplado (0: un tercio del centro).
    pub editor_place: EditorPlace,
    pub editor_rows: u16,
    /// El explorador de archivos, si se quiere ver, y su ancho (0: el de
    /// siempre).
    pub explorer: bool,
    pub explorer_width: u16,
    /// Abierto a mano aunque no quepa: encima de lo demás.
    pub explorer_overlay: bool,
    /// El piano roll está abierto: va en el sitio del tracker.
    pub roll: bool,
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
        self.tabs
            .iter()
            .zip(&short)
            .map(|(t, s)| t.label(*s))
            .collect()
    }
}

/// Dónde va cada cosa.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    /// Toda la pantalla.
    pub screen: Rect,
    pub size: Size,
    pub menu: Option<Rect>,
    pub header: Option<Rect>,
    /// La regla de pasos y la línea de tiempo.
    pub ruler: Option<Rect>,
    /// El explorador de archivos (encima de lo demás si `explorer_overlay`).
    pub explorer: Option<Rect>,
    pub explorer_overlay: bool,
    /// El editor: acoplado abajo, el centro entero o flotando encima
    /// (`floating`).
    pub editor: Option<Rect>,
    pub floating: bool,
    pub editor_place: EditorPlace,
    /// Todo lo que hay entre la cabecera y el mensaje: donde se mueve el
    /// flotante.
    pub body: Option<Rect>,
    pub wave: Option<Rect>,
    /// El tracker o, abierto, el piano roll (si no está en el dock).
    pub tracker: Option<Rect>,
    pub mixer: Option<Rect>,
    pub fx: Option<Rect>,
    pub lanes: Option<Rect>,
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
            Panel::Files => self.explorer,
            // El scope y el espectro del master son siempre pestañas del
            // dock, como la ocupación.
            Panel::Scope | Panel::Spectrum | Panel::Occupancy => None,
        }
    }

    /// Lo que está encima de los paneles (el editor flotante y el
    /// explorador abierto a mano), del de más arriba al de más abajo: el
    /// ratón los mira antes y no llega a lo que tapan.
    pub fn overlays(&self) -> Vec<(Rect, Panel)> {
        let mut out = Vec::new();
        if self.floating
            && let Some(r) = self.editor
        {
            out.push((r, Panel::Code));
        }
        if self.explorer_overlay
            && let Some(r) = self.explorer
        {
            out.push((r, Panel::Files));
        }
        out
    }

    /// Si (`x`, `y`) queda tapado por algo que no es `panel`.
    pub fn covered(&self, panel: Panel, x: u16, y: u16) -> bool {
        let at = ratatui::layout::Position::new(x, y);
        for (r, p) in self.overlays() {
            if r.contains(at) {
                return p != panel;
            }
        }
        false
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
            self.explorer.filter(|_| !self.explorer_overlay),
            self.editor.filter(|_| !self.floating),
            self.wave,
            self.tracker,
            self.mixer,
            self.fx,
            self.lanes,
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

impl Grid {
    /// Sin nada colocado (cada ventana pone lo suyo).
    pub fn empty(screen: Rect) -> Grid {
        Grid {
            screen,
            size: Size::of(screen.width, screen.height),
            menu: None,
            header: None,
            ruler: None,
            explorer: None,
            explorer_overlay: false,
            editor: None,
            floating: false,
            editor_place: EditorPlace::Docked,
            body: None,
            wave: None,
            tracker: None,
            mixer: None,
            fx: None,
            lanes: None,
            reference: None,
            dock: None,
            message: None,
            keys: None,
        }
    }
}

/// La rejilla para una terminal de `screen` con `needs`.
pub fn grid(screen: Rect, needs: &Needs) -> Grid {
    let size = Size::of(screen.width, screen.height);
    let mut g = Grid::empty(screen);
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
    g.body = some(rest);
    // A la izquierda, el explorador (si cabe; si no, encima, si se pidió).
    let centre = explorer(&mut g, rest, needs);
    // Abajo, el editor; arriba, el sonido.
    let sound = editor(&mut g, centre, needs);
    let Some(sound) = sound else {
        return g;
    };
    match size {
        Size::Tiny => unreachable!(),
        Size::Small => small(&mut g, sound, needs),
        Size::Medium => medium(&mut g, sound, needs),
        Size::Wide => wide(&mut g, sound, needs),
    }
    g
}

/// Ancho del explorador por defecto y sus límites.
pub const EXPLORER_WIDTH: u16 = 26;
pub const EXPLORER_MIN: u16 = 14;
/// Desde este ancho de terminal el explorador va a su columna; por debajo
/// se esconde (y se abre encima con su tecla). Es más que el del tamaño
/// mediano: al estrechar, lo primero que se va es el explorador.
pub const EXPLORER_SCREEN: u16 = 120;
/// Alto mínimo del editor acoplado (borde y dos líneas) y lo que deja
/// como mínimo al sonido.
pub const EDITOR_MIN: u16 = 4;
pub const SOUND_MIN: u16 = 6;
/// Tamaño mínimo del flotante.
pub const FLOAT_MIN: (u16, u16) = (20, 5);

/// El ancho que le toca al explorador en un cuerpo de `width` columnas.
pub fn explorer_width(width: u16, wanted: u16) -> u16 {
    let wanted = if wanted == 0 { EXPLORER_WIDTH } else { wanted };
    wanted.clamp(EXPLORER_MIN, (width * 2 / 5).max(EXPLORER_MIN))
}

/// El explorador a la izquierda de `body`: devuelve lo que queda.
fn explorer(g: &mut Grid, body: Rect, needs: &Needs) -> Rect {
    if !needs.explorer && !needs.explorer_overlay {
        return body;
    }
    let width = explorer_width(body.width, needs.explorer_width);
    if needs.explorer && g.screen.width >= EXPLORER_SCREEN {
        let (left, centre) = take_left(body, width);
        g.explorer = some(left);
        return centre;
    }
    if needs.explorer_overlay {
        let (left, _) = take_left(body, width.min(body.width * 3 / 4).max(EXPLORER_MIN));
        g.explorer = some(left);
        g.explorer_overlay = true;
    }
    body
}

/// El alto del editor acoplado en un centro de `height` filas.
pub fn editor_rows(height: u16, wanted: u16) -> u16 {
    // Sin pedir, un tercio (al menos 8, salvo en una terminal muy baja,
    // donde no pasa de dos quintos).
    let wanted = if wanted == 0 {
        (height / 3).max(8.min(height * 2 / 5))
    } else {
        wanted
    };
    let max = height.saturating_sub(SOUND_MIN).max(EDITOR_MIN);
    wanted.clamp(EDITOR_MIN, max).min(height)
}

/// Dónde queda el flotante `wanted` (o el de en medio) dentro de `body`:
/// entero dentro y no más pequeño que [`FLOAT_MIN`].
pub fn float_rect(body: Rect, wanted: Option<Rect>) -> Rect {
    let r = wanted.unwrap_or(Rect {
        x: body.x + body.width / 5,
        y: body.y + body.height / 5,
        width: body.width * 3 / 5,
        height: body.height * 3 / 5,
    });
    let width = r.width.clamp(FLOAT_MIN.0.min(body.width), body.width);
    let height = r.height.clamp(FLOAT_MIN.1.min(body.height), body.height);
    let x = r.x.clamp(body.x, body.right() - width);
    let y = r.y.clamp(body.y, body.bottom() - height);
    Rect {
        x,
        y,
        width,
        height,
    }
}

/// El editor en `centre`: devuelve lo que queda para el sonido (`None`
/// si no queda nada).
fn editor(g: &mut Grid, centre: Rect, needs: &Needs) -> Option<Rect> {
    if !needs.editor {
        return some(centre);
    }
    g.editor_place = needs.editor_place;
    match needs.editor_place {
        EditorPlace::Docked => {
            let rows = editor_rows(centre.height, needs.editor_rows);
            let (sound, editor) = take_bottom(centre, rows);
            g.editor = some(editor);
            some(sound)
        }
        EditorPlace::Maximized => {
            g.editor = some(centre);
            None
        }
        EditorPlace::Floating(wanted) => {
            let body = g.body.unwrap_or(centre);
            g.editor = some(float_rect(body, wanted));
            g.floating = g.editor.is_some();
            some(centre)
        }
    }
}

/// La onda en pocas filas y las pestañas con todo lo demás.
fn small(g: &mut Grid, sound: Rect, needs: &Needs) {
    // La onda: bordes y dos filas (una si no hay alto).
    let wave = if sound.height >= 14 {
        6
    } else if sound.height >= 7 {
        4
    } else {
        MIN_PANEL
    };
    let (wave, rest) = take_top(sound, wave);
    g.wave = some(wave);
    let mut tabs = vec![
        Tab::Panel(Panel::Tracker),
        Tab::Panel(Panel::Mixer),
        Tab::Panel(Panel::Fx),
        Tab::Panel(Panel::Lanes),
    ];
    master_tabs(needs, &mut tabs);
    extra_tabs(needs, &mut tabs);
    g.dock = dock(rest, tabs);
}

/// Alto que necesita el sonido para colocarse como en el tamaño mediano
/// y en el ancho; con menos (el editor ha crecido), como en el de abajo.
const MEDIUM_SOUND: u16 = 20;
const WIDE_SOUND: u16 = 26;

/// El scope y el espectro del master, si se han pedido: dos pestañas más
/// (en todos los tamaños; fuera de la que se ve, ni se calculan).
fn master_tabs(needs: &Needs, tabs: &mut Vec<Tab>) {
    if needs.master_viz {
        tabs.push(Tab::Panel(Panel::Spectrum));
        tabs.push(Tab::Panel(Panel::Scope));
    }
}

/// La onda, el tracker con el mezclador al lado y, debajo, las pestañas.
fn medium(g: &mut Grid, sound: Rect, needs: &Needs) {
    // Con poco alto (el editor crecido), como en el pequeño.
    if sound.height < MEDIUM_SOUND {
        return small(g, sound, needs);
    }
    let dock_min = 6;
    let wave = if needs.editor { 8 } else { sound.height / 2 };
    let (wave, seq, dock_area) = stack(sound, wave, dock_min, needs.dock_want);
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
    master_tabs(needs, &mut tabs);
    extra_tabs(needs, &mut tabs);
    g.dock = dock(dock_area, tabs);
}

/// La onda, el tracker entero y las pestañas; a la derecha, el
/// mezclador y la referencia de teclas.
fn wide(g: &mut Grid, sound: Rect, needs: &Needs) {
    if sound.height < WIDE_SOUND {
        return medium(g, sound, needs);
    }
    let (left, right) = take_right(sound, WIDE_RIGHT);
    // Derecha: el mezclador arriba y la referencia con lo que quede.
    let mixer = (right.height / 2).clamp(MIN_PANEL, 16);
    let (mixer, r) = take_top(right, mixer);
    g.mixer = some(mixer);
    g.reference = some(r);
    // Centro: la onda, el tracker entero si cabe y las pestañas abajo.
    let wave = if needs.editor { 10 } else { 14 };
    let (wave, seq, dock_area) = stack(left, wave, 6, needs.dock_want);
    g.wave = some(wave);
    g.tracker = some(seq);
    let mut tabs = vec![Tab::Panel(Panel::Fx), Tab::Panel(Panel::Lanes)];
    master_tabs(needs, &mut tabs);
    extra_tabs(needs, &mut tabs);
    g.dock = dock(dock_area, tabs);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn needs() -> Needs {
        Needs {
            editor: true,
            explorer: true,
            mixer_width: 26,
            menu: true,
            master_viz: true,
            ..Default::default()
        }
    }

    /// Nada se solapa ni se sale de la pantalla (lo que flota, dentro del
    /// cuerpo).
    fn check(screen: Rect, g: &Grid) {
        let rects = g.rects();
        for (i, a) in rects.iter().enumerate() {
            assert!(
                screen.contains(a.as_position())
                    && a.right() <= screen.right()
                    && a.bottom() <= screen.bottom(),
                "{a:?} se sale de {screen:?}"
            );
            for b in &rects[i + 1..] {
                assert!(!a.intersects(*b), "{a:?} y {b:?} se solapan en {screen:?}");
            }
        }
        for (r, _) in g.overlays() {
            let body = g.body.unwrap();
            assert!(
                r.x >= body.x
                    && r.y >= body.y
                    && r.right() <= body.right()
                    && r.bottom() <= body.bottom(),
                "{r:?} fuera de {body:?}"
            );
        }
    }

    const SIZES: [(u16, u16); 13] = [
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
        (300, 12),
    ];

    #[test]
    fn no_size_overlaps_or_spills() {
        let places = [
            EditorPlace::Docked,
            EditorPlace::Maximized,
            EditorPlace::Floating(None),
            EditorPlace::Floating(Some(Rect::new(500, 500, 900, 900))),
        ];
        for (w, h) in SIZES {
            let screen = Rect::new(0, 0, w, h);
            for editor in [true, false] {
                for roll in [true, false] {
                    for explorer in [true, false] {
                        for place in places {
                            let n = Needs {
                                editor,
                                roll,
                                explorer,
                                explorer_overlay: !explorer,
                                editor_place: place,
                                occupancy: true,
                                sidechain: true,
                                ..needs()
                            };
                            check(screen, &grid(screen, &n));
                        }
                    }
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
        assert_eq!(
            dock(60).labels(),
            ["3 tracker", "4 mezcla", "5 efectos", "6 lanes"]
        );
        // A 40 basta con acortar el más largo.
        assert_eq!(
            dock(40).labels(),
            ["3 trk", "4 mezcla", "5 efectos", "6 lanes"]
        );
        assert_eq!(dock(36).labels(), ["3 trk", "4 mezcla", "5 fx", "6 lanes"]);
        assert_eq!(dock(20).labels(), ["3 trk", "4 mez", "5 fx", "6 lan"]);
    }

    #[test]
    fn the_dock_grows_to_what_its_tab_asks_for() {
        let screen = Rect::new(0, 0, 180, 50);
        let plain = grid(screen, &needs());
        let tall = grid(
            screen,
            &Needs {
                dock_want: 14,
                ..needs()
            },
        );
        let (a, b) = (plain.dock.unwrap().area, tall.dock.unwrap().area);
        assert!(b.height > a.height, "{a:?} {b:?}");
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
        let g = grid(Rect::new(0, 0, 39, 9), &needs());
        assert!(g.header.is_some() && g.message.is_some());
        assert!(g.editor.is_none() && g.wave.is_none() && g.dock.is_none());
        assert!(g.explorer.is_none());
    }

    #[test]
    fn small_hides_the_explorer_and_puts_the_editor_below() {
        let g = grid(Rect::new(0, 0, 80, 24), &needs());
        assert_eq!(g.size, Size::Small);
        assert!(
            g.menu.is_none(),
            "a 80 columnas la barra de menús no ocupa fila"
        );
        assert!(g.explorer.is_none(), "a 80 columnas no cabe el explorador");
        let (editor, wave) = (g.editor.unwrap(), g.wave.unwrap());
        assert_eq!((editor.x, editor.width), (0, 80));
        assert_eq!(editor.bottom(), g.message.unwrap().y);
        assert!(wave.bottom() <= editor.y);
        assert!(g.tracker.is_none() && g.mixer.is_none());
        let dock = g.dock.as_ref().unwrap();
        assert_eq!(dock.tabs[0], Tab::Panel(Panel::Tracker));
        assert!(dock.tabs.contains(&Tab::Panel(Panel::Spectrum)));
        assert_eq!(dock.area.bottom(), editor.y);
        // Cabecera, regla, cuerpo, mensaje y teclas: 24 filas.
        assert_eq!(g.header.unwrap().y, 0);
        assert_eq!(g.ruler.unwrap().y, 1);
        assert_eq!(g.keys.unwrap().y, 23);
        assert_eq!(g.message.unwrap().y, 22);
        // Abierto a mano, encima.
        let g = grid(
            Rect::new(0, 0, 80, 24),
            &Needs {
                explorer_overlay: true,
                ..needs()
            },
        );
        let files = g.explorer.unwrap();
        assert!(g.explorer_overlay && files.x == 0 && files.y == 2);
        assert!(g.covered(Panel::Code, files.x + 1, g.editor.unwrap().y + 1));
        assert!(!g.covered(Panel::Files, files.x + 1, files.y + 1));
    }

    #[test]
    fn small_ascii_60_by_16_keeps_the_wave_the_tabs_and_the_editor() {
        let g = grid(Rect::new(0, 0, 60, 16), &needs());
        assert!(g.explorer.is_none());
        let (wave, dock, editor) = (g.wave.unwrap(), g.dock.unwrap().area, g.editor.unwrap());
        assert!(wave.height >= MIN_PANEL && dock.height >= 1);
        assert!(editor.height >= EDITOR_MIN);
        assert_eq!(editor.bottom(), g.message.unwrap().y);
    }

    #[test]
    fn medium_has_the_explorer_left_the_tracker_beside_the_mixer_and_the_tabs_below() {
        let g = grid(Rect::new(0, 0, 120, 40), &needs());
        assert_eq!(g.size, Size::Medium);
        assert!(g.menu.is_some());
        let files = g.explorer.unwrap();
        assert_eq!((files.x, files.width), (0, EXPLORER_WIDTH));
        assert_eq!(files.bottom(), g.message.unwrap().y);
        let editor = g.editor.unwrap();
        assert_eq!(editor.x, files.right());
        assert_eq!(editor.right(), 120);
        assert_eq!(editor.height, editor_rows(files.height, 0));
        let (tracker, mixer) = (g.tracker.unwrap(), g.mixer.unwrap());
        assert_eq!(tracker.right(), mixer.x);
        assert_eq!(mixer.width, 26);
        assert_eq!(g.wave.unwrap().x, files.right());
        // Abajo, las pestañas a todo el ancho; el scope y el espectro del
        // master, pedidos, son dos pestañas más (nunca un panel fijo).
        let dock = g.dock.as_ref().unwrap().area;
        assert_eq!((dock.x, dock.right()), (files.right(), 120));
        assert_eq!(dock.bottom(), editor.y);
        assert!(g.area_of(Panel::Spectrum).is_none() && g.area_of(Panel::Scope).is_none());
        assert!(g.docked(Tab::Panel(Panel::Fx)) && g.docked(Tab::Panel(Panel::Scope)));
        assert!(g.docked(Tab::Panel(Panel::Spectrum)));
        // Sin pedirlos, ni pestañas.
        let off = grid(
            Rect::new(0, 0, 120, 40),
            &Needs {
                master_viz: false,
                ..needs()
            },
        );
        assert!(!off.docked(Tab::Panel(Panel::Scope)) && !off.docked(Tab::Panel(Panel::Spectrum)));
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
        // Sin explorador, el centro empieza en 0.
        let g = grid(
            Rect::new(0, 0, 120, 40),
            &Needs {
                explorer: false,
                ..needs()
            },
        );
        assert!(g.explorer.is_none());
        assert_eq!(g.editor.unwrap().x, 0);
        assert_eq!(g.wave.unwrap().x, 0);
    }

    #[test]
    fn wide_has_the_explorer_the_sound_and_a_column_with_the_mixer_and_the_reference() {
        let g = grid(Rect::new(0, 0, 180, 50), &needs());
        assert_eq!(g.size, Size::Wide);
        let (files, wave, mixer) = (g.explorer.unwrap(), g.wave.unwrap(), g.mixer.unwrap());
        assert!(files.right() <= wave.x && wave.right() <= mixer.x);
        assert_eq!(mixer.width, WIDE_RIGHT);
        // El scope y el espectro del master, pedidos, en las pestañas; la
        // referencia de teclas debajo del mezclador.
        assert!(g.area_of(Panel::Scope).is_none() && g.area_of(Panel::Spectrum).is_none());
        assert!(g.docked(Tab::Panel(Panel::Spectrum)) && g.docked(Tab::Panel(Panel::Scope)));
        assert_eq!(g.reference.unwrap().y, mixer.bottom());
        assert!(g.tracker.unwrap().height >= TRACKER_FULL - 3);
        // Con el editor algo más bajo, el tracker entero.
        let low = grid(
            Rect::new(0, 0, 180, 50),
            &Needs {
                editor_rows: 12,
                ..needs()
            },
        );
        assert_eq!(low.tracker.unwrap().height, TRACKER_FULL);
        assert!(g.docked(Tab::Panel(Panel::Fx)) && g.docked(Tab::Panel(Panel::Lanes)));
        let editor = g.editor.unwrap();
        assert_eq!((editor.x, editor.right()), (files.right(), 180));
        assert!(mixer.bottom() <= editor.y);
    }

    #[test]
    fn the_editor_height_follows_what_is_asked_within_its_limits() {
        let screen = Rect::new(0, 0, 120, 40);
        let at = |rows| {
            grid(
                screen,
                &Needs {
                    editor_rows: rows,
                    ..needs()
                },
            )
        };
        assert_eq!(at(12).editor.unwrap().height, 12);
        let body = at(0).explorer.unwrap().height;
        assert_eq!(at(1).editor.unwrap().height, EDITOR_MIN);
        assert_eq!(at(200).editor.unwrap().height, body - SOUND_MIN);
        // Al crecer el editor, el sonido se aprieta.
        assert!(at(20).wave.unwrap().y == at(12).wave.unwrap().y);
        assert!(at(20).dock.unwrap().area.bottom() < at(12).dock.unwrap().area.bottom());
    }

    #[test]
    fn a_tall_editor_moves_the_sound_to_the_tabs() {
        let screen = Rect::new(0, 0, 120, 40);
        let g = grid(
            screen,
            &Needs {
                editor_rows: 22,
                ..needs()
            },
        );
        // Con poco alto, como en el pequeño: el tracker es una pestaña.
        assert!(g.tracker.is_none() && g.area_of(Panel::Spectrum).is_none());
        assert!(g.docked(Tab::Panel(Panel::Tracker)));
        assert!(g.docked(Tab::Panel(Panel::Spectrum)));
        let g = grid(
            Rect::new(0, 0, 180, 50),
            &Needs {
                editor_rows: 24,
                ..needs()
            },
        );
        // El ancho pasa a ser como el mediano.
        assert!(g.area_of(Panel::Scope).is_none() && g.tracker.is_some());
    }

    #[test]
    fn maximized_the_editor_takes_the_centre() {
        let g = grid(
            Rect::new(0, 0, 120, 40),
            &Needs {
                editor_place: EditorPlace::Maximized,
                ..needs()
            },
        );
        let (files, editor) = (g.explorer.unwrap(), g.editor.unwrap());
        assert_eq!(editor.x, files.right());
        assert_eq!((editor.y, editor.height), (files.y, files.height));
        assert!(g.wave.is_none() && g.tracker.is_none() && g.dock.is_none());
    }

    #[test]
    fn floating_the_editor_covers_the_rest_and_stays_inside() {
        let screen = Rect::new(0, 0, 120, 40);
        let float = |at| {
            grid(
                screen,
                &Needs {
                    editor_place: EditorPlace::Floating(at),
                    ..needs()
                },
            )
        };
        let g = float(None);
        let editor = g.editor.unwrap();
        assert!(g.floating);
        // El sonido llena el centro, como sin editor.
        let docked = grid(screen, &needs());
        assert!(
            g.dock.as_ref().unwrap().area.bottom() > docked.dock.as_ref().unwrap().area.bottom()
        );
        // Tapa lo de debajo.
        let wave = g.wave.unwrap();
        let inside = (editor.x + 1, editor.y + 1);
        assert!(g.covered(Panel::Wave, inside.0, inside.1) || !wave.contains(inside.into()));
        assert!(!g.covered(Panel::Code, inside.0, inside.1));
        assert!(!g.covered(Panel::Wave, wave.x, wave.y));
        // Donde se deja, sin salirse ni hacerse diminuto.
        let g = float(Some(Rect::new(100, 30, 60, 30)));
        let r = g.editor.unwrap();
        let body = g.body.unwrap();
        assert_eq!((r.right(), r.bottom()), (body.right(), body.bottom()));
        let g = float(Some(Rect::new(10, 10, 2, 1)));
        assert_eq!(
            (g.editor.unwrap().width, g.editor.unwrap().height),
            FLOAT_MIN
        );
    }

    #[test]
    fn the_explorer_width_follows_what_is_asked_within_its_limits() {
        let screen = Rect::new(0, 0, 120, 40);
        let at = |w| {
            grid(
                screen,
                &Needs {
                    explorer_width: w,
                    ..needs()
                },
            )
            .explorer
            .unwrap()
            .width
        };
        assert_eq!(at(30), 30);
        assert_eq!(at(3), EXPLORER_MIN);
        assert_eq!(at(200), 120 * 2 / 5);
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
            let left = g.explorer.map_or(0, |r| r.right());
            assert_eq!(g.wave.unwrap().x, left, "{w}×{h}");
            assert_eq!(
                g.dock.unwrap().area.bottom().max(g.wave.unwrap().bottom()),
                g.message.unwrap().y,
                "{w}×{h}"
            );
        }
    }
}
