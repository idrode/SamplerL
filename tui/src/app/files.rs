//! El explorador de archivos en la aplicación (ver [`crate::files`] y
//! [`crate::files_panel`]): mostrarlo y esconderlo, sus teclas con el foco
//! (Alt-0), el ratón y lo que hace con cada archivo:
//!
//! - un **`.lua`** se abre en el editor (si hay cambios sin guardar, hay
//!   que pedirlo dos veces) y se carga, como al arrancar con él;
//! - un **`.sampler`** se abre como proyecto (la orden `abrir`);
//! - un **sample** se preescucha en la voz aparte (la del explorador de
//!   samples, la misma caché) y se ve en el panel del sample y al pie del
//!   árbol; `a` escribe su `sample("...")` en el cursor del editor;
//! - una **carpeta** se abre o se cierra (se lee al abrirla).
//!
//! También crea (`n`), renombra (`r`) y borra (`Supr`, con confirmación)
//! archivos; las carpetas no se borran desde aquí. Todo en el hilo de la
//! interfaz: el árbol solo lee la carpeta que se abre.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ratatui::layout::Rect;
use ratatui_textarea::{Input, Key, TextArea};

use super::{App, Flow, MouseAction};
use crate::files::{self, Kind, Tree};
use crate::files_panel::{FilesHit, FilesLayout, FilesView, Preview};
use crate::focus::Panel;
use crate::model::Waveform;

/// La carpeta de samples de un proyecto, junto a su script.
pub const PROJECT_SAMPLES: &str = "samples";
/// Dos clics en la misma fila en menos de esto: doble clic (abre).
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// Columnas que cambia el ancho cada `<` o `>` (y Ctrl-Alt-←→).
pub(super) const WIDTH_STEP: u16 = 2;

/// Lo que espera una respuesta en la línea de abajo del explorador.
#[derive(Debug, Clone, PartialEq)]
enum Ask {
    /// El nombre de un archivo nuevo en esa carpeta.
    New(PathBuf),
    /// El nombre nuevo de ese archivo.
    Rename(PathBuf),
    /// ¿Borrar ese archivo? (`s` o Enter, sí; otra tecla, no).
    Delete(PathBuf),
}

/// El estado del explorador.
#[derive(Debug)]
pub(crate) struct Explorer {
    /// El árbol (se crea la primera vez que se ve).
    tree: Option<Tree>,
    /// Se quiere ver en su columna (`[interfaz] explorador`).
    pub(super) shown: bool,
    /// Abierto a mano encima de lo demás (terminal estrecha).
    pub(super) overlay: bool,
    /// Su ancho (0: el de siempre; `[interfaz] ancho_explorador`).
    pub(super) width: u16,
    ask: Option<(Ask, String)>,
    /// Un `.lua` pedido con cambios sin guardar: otra vez lo abre.
    open_armed: Option<PathBuf>,
    /// La vista previa del último sample preescuchado.
    preview: Option<Preview>,
    last_click: Option<(Instant, usize)>,
    /// Dónde se dibujó (el ratón lo usa) y si se arrastra su borde.
    layout: Option<FilesLayout>,
    drag: bool,
    /// El script del editor tal como se abrió y su ruta completa (la del
    /// árbol); se recalcula solo si cambia.
    current: Option<(PathBuf, PathBuf)>,
    /// La carpeta de samples global (`[samples] carpeta`).
    global: Option<PathBuf>,
    /// Los marcados con `m` para añadirlos juntos.
    marked: Vec<PathBuf>,
    /// La copia en marcha al proyecto.
    adding: Option<Adding>,
    /// Lo que se arrastra del árbol con el ratón.
    dragging: Option<PathBuf>,
}

/// Una copia al proyecto en marcha y lo que hay que hacer al acabar.
pub(crate) struct Adding {
    running: crate::add_samples::Running,
    /// Escribir `sample("...")` del (único) añadido en el cursor.
    insert: bool,
    /// De «Recoger»: cada texto de `sample("...")` y su archivo de fuera.
    gather: Vec<(String, PathBuf)>,
    /// Lo que se dejó al preparar la copia.
    skipped: Vec<String>,
    /// Lo último que se enseñó de la barra (para no redibujar sin cambio).
    shown: (u64, usize),
}

impl std::fmt::Debug for Adding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Adding")
            .field("files", &self.running.files)
            .finish()
    }
}

impl Default for Explorer {
    fn default() -> Self {
        Explorer {
            tree: None,
            shown: true,
            overlay: false,
            width: 0,
            ask: None,
            open_armed: None,
            preview: None,
            last_click: None,
            layout: None,
            drag: false,
            current: None,
            global: None,
            marked: Vec::new(),
            adding: None,
            dragging: None,
        }
    }
}

impl App {
    /// El explorador se ve (en su columna o encima).
    fn files_visible(&self) -> bool {
        self.grid.as_ref().is_some_and(|g| g.explorer.is_some())
    }

    /// Crea el árbol la primera vez que hace falta (o tras cambiar sus
    /// raíces), con el script elegido.
    fn ensure_files(&mut self) {
        if self.files.tree.is_none() {
            let mut tree = Tree::with_roots(&self.files_roots());
            tree.reveal(&crate::target::full_path(&self.path));
            self.files.tree = Some(tree);
        }
    }

    /// La carpeta del script (la de un `song.lua` a secas, `.`).
    pub(super) fn script_dir(&self) -> PathBuf {
        self.path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .to_path_buf()
    }

    /// La carpeta de samples del proyecto: `samples/` junto al script.
    pub(super) fn project_samples(&self) -> PathBuf {
        self.script_dir().join(PROJECT_SAMPLES)
    }

    /// Las raíces del árbol, en orden: la carpeta de samples del proyecto
    /// (`samples/` junto al script, si existe, y la de `samples =` del
    /// `.sampler` si es otra), la global (`[samples] carpeta`) y la del
    /// proyecto o el script. Solo mira si existe `samples/`: no lee nada.
    pub(super) fn files_roots(&self) -> Vec<files::Root> {
        let root = |label: &str, dir: PathBuf| files::Root {
            label: label.into(),
            dir,
        };
        let mut roots = Vec::new();
        let samples = self.project_samples();
        if samples.is_dir() {
            roots.push(root("proyecto", samples));
        }
        if let Some(dir) = self.host.sample_dir()
            && crate::target::full_path(dir) != crate::target::full_path(&self.base_dir())
        {
            roots.push(root("proyecto", dir.to_path_buf()));
        }
        if let Some(dir) = &self.files.global {
            roots.push(root("global", dir.clone()));
        }
        roots.push(root("script", self.base_dir()));
        roots
    }

    /// Las raíces que tocan ya no son las del árbol.
    fn files_roots_changed(&self) -> bool {
        let Some(tree) = &self.files.tree else {
            return false;
        };
        let mut dirs: Vec<PathBuf> = Vec::new();
        for root in self.files_roots() {
            let dir = crate::target::full_path(&root.dir);
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        tree.root_dirs() != dirs
    }

    /// Las raíces del árbol cambian (otro script o proyecto, otra carpeta
    /// global, `samples/` recién creada): se vuelve a hacer al verse.
    pub(super) fn reset_files(&mut self) {
        self.files.tree = None;
    }

    /// La carpeta de samples global (`[samples] carpeta`).
    pub(super) fn set_global_samples(&mut self, dir: Option<PathBuf>) {
        self.host.set_global_sample_dir(dir.clone());
        if self.files.global != dir {
            self.files.global = dir;
            self.reset_files();
        }
    }

    /// Lo que dibuja el panel en `area` (lo llama el dibujo; `overlay`:
    /// encima de lo demás): desplaza el árbol para que se vea la fila
    /// elegida y apunta dónde quedó. Si cabe en su columna, deja de ir
    /// encima (al volver a estrechar la terminal no reaparece solo).
    pub fn place_files(&mut self, area: Rect, overlay: bool) -> FilesView {
        if !overlay {
            self.files.overlay = false;
        }
        self.ensure_files();
        if self.files.current.as_ref().map(|c| &c.0) != Some(&self.path) {
            let full = crate::target::full_path(&self.path);
            self.files.current = Some((self.path.clone(), full));
        }
        let mut view = self.files_view();
        let layout = FilesLayout::new(&view, area);
        if let Some(tree) = &mut self.files.tree {
            tree.scroll_to(layout.tree.height as usize);
            view.top = tree.top;
        }
        self.files.layout = Some(FilesLayout {
            top: view.top,
            ..layout
        });
        view
    }

    /// Las filas del árbol (en los tests).
    #[cfg(test)]
    pub fn view_files_rows(&self) -> Vec<files::Row> {
        self.files.tree.as_ref().map(Tree::rows).unwrap_or_default()
    }

    /// Los avisos de la última carga (en los tests).
    #[cfg(test)]
    pub fn host_warnings_for_test(&self) -> Vec<String> {
        self.host
            .song()
            .map(|s| s.warnings.clone())
            .unwrap_or_default()
    }

    /// La copia al proyecto acabó (en los tests).
    #[cfg(test)]
    pub fn adding_done(&self) -> bool {
        self.files.adding.is_none()
    }

    /// Elige la fila `i` del árbol (en los tests).
    #[cfg(test)]
    pub fn select_file_for_test(&mut self, i: usize) {
        if let Some(tree) = &mut self.files.tree {
            tree.select(i);
        }
    }

    #[cfg(test)]
    pub fn files_layout_for_test(&self) -> Option<FilesLayout> {
        self.files.layout
    }

    /// El dibujo no puso el explorador (no cabe o está escondido).
    pub fn clear_files_layout(&mut self) {
        self.files.layout = None;
    }

    fn files_view(&self) -> FilesView {
        let tree = self.files.tree.as_ref();
        FilesView {
            rows: tree.map(Tree::rows).unwrap_or_default(),
            selected: tree.map_or(0, |t| t.selected),
            top: tree.map_or(0, |t| t.top),
            current: self.files.current.as_ref().map(|c| c.1.clone()),
            focused: self.chosen_focus == Some(Panel::Files),
            input: self.files.ask.as_ref().map(|(ask, text)| {
                let label = match ask {
                    Ask::New(_) => "nuevo: ".to_string(),
                    Ask::Rename(_) => "nombre: ".to_string(),
                    Ask::Delete(p) => format!(
                        "¿borrar {}? s/n ",
                        p.file_name().unwrap_or_default().to_string_lossy()
                    ),
                };
                (label, text.clone())
            }),
            preview: self.files.preview.clone(),
            marked: self.files.marked.clone(),
        }
    }

    /// Lo que la vista necesita del explorador: si se quiere ver, si va
    /// encima, su ancho y la vista previa (con el foco en él).
    pub(super) fn files_needs(&self) -> (bool, bool, u16, Option<Preview>) {
        let preview = self
            .files
            .preview
            .clone()
            .filter(|_| self.chosen_focus == Some(Panel::Files));
        (
            self.files.shown,
            self.files.overlay,
            self.files.width,
            preview,
        )
    }

    /// «Explorador de archivos» (Alt-E): lo esconde si se ve en su
    /// columna; en una terminal estrecha, lo abre o cierra encima; si
    /// estaba escondido, lo vuelve a poner.
    pub fn toggle_explorer(&mut self) {
        let docked = self
            .grid
            .as_ref()
            .is_some_and(|g| g.explorer.is_some() && !g.explorer_overlay);
        let narrow = self
            .grid
            .as_ref()
            .is_some_and(|g| g.screen.width < crate::layout::EXPLORER_SCREEN);
        if docked {
            self.files.shown = false;
            self.leave_files();
            self.message = "explorador escondido (Alt-E lo vuelve a poner)".into();
        } else if narrow {
            self.files.shown = true;
            self.files.overlay = !self.files.overlay;
            if self.files.overlay {
                self.chosen_focus = Some(Panel::Files);
                self.message = files_keys().into();
            } else {
                self.leave_files();
            }
        } else {
            self.files.shown = true;
            self.message = "explorador de archivos (Alt-0 le da el foco)".into();
        }
        self.persist_layout();
    }

    /// Alt-0: el foco al explorador (si no se veía, aparece).
    pub(super) fn focus_files(&mut self) {
        let narrow = self
            .grid
            .as_ref()
            .is_some_and(|g| g.screen.width < crate::layout::EXPLORER_SCREEN);
        if !self.files.shown || narrow {
            self.files.shown = true;
            self.files.overlay = narrow;
        }
        self.ensure_files();
        self.chosen_focus = Some(Panel::Files);
        self.message = files_keys().into();
    }

    /// Suelta el foco del explorador (y cierra el de encima).
    fn leave_files(&mut self) {
        if self.chosen_focus == Some(Panel::Files) {
            self.chosen_focus = None;
        }
        self.files.overlay = false;
        self.files.ask = None;
    }

    /// Cambia el ancho del explorador `delta` columnas.
    pub(super) fn resize_explorer(&mut self, delta: i32) {
        let body = self
            .grid
            .as_ref()
            .and_then(|g| g.body)
            .map_or(120, |b| b.width);
        let now = crate::layout::explorer_width(body, self.files.width) as i32;
        self.files.width = crate::layout::explorer_width(body, (now + delta).max(1) as u16);
        self.persist_layout();
    }

    /// Una tecla con el foco en el explorador: `None` si no es suya (las
    /// de Ctrl, Alt y F siguen su camino).
    pub(super) fn handle_files(&mut self, input: &Input) -> Option<Flow> {
        if self.chosen_focus != Some(Panel::Files) {
            return None;
        }
        if self.files.ask.is_some() {
            self.files_ask_key(input);
            return Some(Flow::Continue);
        }
        if input.ctrl || input.alt || matches!(input.key, Key::F(_)) {
            return None;
        }
        self.ensure_files();
        let row = self.files.tree.as_ref().and_then(Tree::selected_row);
        let page = self
            .files
            .layout
            .map_or(10, |l| l.tree.height.max(1) as isize);
        match input.key {
            Key::Esc => {
                self.leave_files();
                self.message = String::new();
            }
            Key::Up | Key::Char('k') => self.files_step(-1),
            Key::Down | Key::Char('j') => self.files_step(1),
            Key::PageUp => self.files_step(-page),
            Key::PageDown => self.files_step(page),
            Key::Home => self.files_step(isize::MIN / 2),
            Key::End | Key::Char('G') => self.files_step(isize::MAX / 2),
            Key::Left | Key::Char('h') => {
                if let Some(tree) = &mut self.files.tree {
                    tree.close_or_parent();
                }
            }
            Key::Right | Key::Char('l') => match row {
                Some(r) if r.kind == Kind::Dir && !r.note => {
                    let i = self.files.tree.as_ref().map_or(0, |t| t.selected);
                    if let Some(tree) = &mut self.files.tree
                        && !tree.open(i)
                    {
                        tree.step(1);
                    }
                }
                Some(_) => self.activate_file(),
                None => {}
            },
            Key::Enter => self.activate_file(),
            Key::Char(' ') => match row {
                Some(r) if r.kind == Kind::Audio => self.preview_tree_file(&r.path),
                _ => self.activate_file(),
            },
            Key::Char('a') => self.add_to_project(),
            Key::Char('A') => self.reference_sample(),
            Key::Char('m') => self.toggle_mark(),
            Key::Char('n') => {
                let dir = match &row {
                    Some(r) if r.kind == Kind::Dir => r.path.clone(),
                    Some(r) => r.path.parent().unwrap_or(Path::new(".")).to_path_buf(),
                    None => self.base_dir(),
                };
                self.files.ask = Some((Ask::New(dir), String::new()));
            }
            Key::Char('r') => match row {
                Some(r) if r.kind != Kind::Dir && !r.note => {
                    self.files.ask = Some((Ask::Rename(r.path.clone()), r.name.clone()));
                }
                _ => {
                    self.message = "r: elige un archivo (las carpetas no se renombran aquí)".into()
                }
            },
            Key::Delete | Key::Char('d') => match row {
                Some(r) if r.kind != Kind::Dir && !r.note => {
                    if self.is_current_script(&r.path) {
                        self.message = "es el script del editor: no se borra desde aquí".into();
                    } else {
                        self.files.ask = Some((Ask::Delete(r.path.clone()), String::new()));
                    }
                }
                _ => {
                    self.message = "Supr: elige un archivo (las carpetas no se borran aquí)".into()
                }
            },
            Key::Char('R') => self.refresh_files(),
            Key::Char('<') | Key::Char('-') => self.resize_explorer(-(WIDTH_STEP as i32)),
            Key::Char('>') | Key::Char('+') => self.resize_explorer(WIDTH_STEP as i32),
            _ => {}
        }
        Some(Flow::Continue)
    }

    /// «Refrescar el explorador» (R con el foco): vuelve a leer las
    /// carpetas abiertas (las cerradas no se leen).
    pub(super) fn refresh_files(&mut self) {
        // Si cambió qué raíces hay (se creó `samples/`), el árbol de nuevo;
        // si no, solo lo abierto.
        if self.files_roots_changed() {
            self.reset_files();
        } else if let Some(tree) = &mut self.files.tree {
            tree.refresh(None);
        }
        self.message = "explorador: vuelto a leer".into();
    }

    fn files_step(&mut self, delta: isize) {
        if let Some(tree) = &mut self.files.tree {
            tree.step(delta);
        }
        self.files.open_armed = None;
    }

    /// `path` es el script del editor (con la ruta que sea).
    fn is_current_script(&self, path: &Path) -> bool {
        path == self.path || crate::target::full_path(path) == crate::target::full_path(&self.path)
    }

    /// Enter en la fila elegida.
    fn activate_file(&mut self) {
        let Some(row) = self.files.tree.as_ref().and_then(Tree::selected_row) else {
            return;
        };
        if row.note {
            return;
        }
        match row.kind {
            Kind::Dir => {
                let i = self.files.tree.as_ref().map_or(0, |t| t.selected);
                if let Some(tree) = &mut self.files.tree
                    && tree.can_toggle(i)
                {
                    tree.toggle(i);
                }
            }
            Kind::Script => self.open_script(&row.path),
            Kind::Project => {
                let rel = relative(&row.path, &self.base_dir());
                self.run_command(&format!("abrir {rel}"));
                if let Some(tree) = &mut self.files.tree {
                    tree.refresh(None);
                }
            }
            Kind::Audio => self.preview_tree_file(&row.path),
            Kind::Unsupported => self.message = unsupported_message(&row.path),
        }
    }

    /// Abre `path` en el editor y lo carga (con cambios sin guardar, hay
    /// que pedirlo otra vez).
    pub(super) fn open_script(&mut self, path: &Path) {
        if self.is_current_script(path) {
            self.message = "ya está en el editor".into();
            return;
        }
        if self.is_main_window() {
            self.message = "el editor está en su ventana: ábrelo allí (o vuelve a meterlo)".into();
            return;
        }
        if self.dirty() && self.files.open_armed.as_deref() != Some(path) {
            self.files.open_armed = Some(path.to_path_buf());
            self.message = format!(
                "hay cambios sin guardar en {}: Enter otra vez abre {} y los descarta (o Ctrl-S antes)",
                self.file_name(),
                path.file_name().unwrap_or_default().to_string_lossy()
            );
            return;
        }
        self.files.open_armed = None;
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) => {
                self.message = format!("no se pudo abrir {}: {e}", path.display());
                return;
            }
        };
        self.editor = TextArea::new(text.lines().map(String::from).collect());
        self.saved = text;
        self.path = path.to_path_buf();
        self.external = None;
        self.overwrite_armed = false;
        self.undo_groups.clear();
        self.redo_groups.clear();
        self.completion = None;
        self.marks.clear();
        // Otro script, fuera del proyecto: sus samples, junto a él.
        if self.project.is_some() {
            self.project = None;
            self.host.set_sample_dir(None);
        }
        // Un script de fuera de las raíces: el árbol de nuevo, con las suyas
        // (uno de dentro, el árbol de siempre).
        let full = crate::target::full_path(path);
        if let Some(tree) = &self.files.tree
            && !tree.root_dirs().iter().any(|dir| full.starts_with(dir))
        {
            self.reset_files();
        }
        self.load();
        if self.error.is_none() {
            self.message = format!("{} abierto: entra en el siguiente compás", self.file_name());
        }
    }

    /// Preescucha el sample de `path` y lo deja a la vista.
    fn preview_tree_file(&mut self, path: &Path) {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        match self.preview_file(path) {
            Ok(sample) => {
                let seconds = sample.frames() as f64 / sample.sample_rate().max(1) as f64;
                self.files.preview = Some(Preview {
                    name: name.clone(),
                    info: format!("{seconds:.2} s"),
                    waveform: Arc::new(Waveform::of(&sample)),
                });
                self.message =
                    format!("{name} · {seconds:.2} s · a escribe sample(\"...\") en el editor");
            }
            Err(e) => {
                self.files.preview = None;
                self.message = format!("{name}: {e}");
            }
        }
    }

    /// `m`: marca o desmarca la fila elegida (un sample o una carpeta)
    /// para añadirla con las demás.
    fn toggle_mark(&mut self) {
        let Some(row) = self.files.tree.as_ref().and_then(Tree::selected_row) else {
            return;
        };
        if row.note || !matches!(row.kind, Kind::Audio | Kind::Dir) {
            self.message = "m: marca samples o carpetas para añadirlos juntos".into();
            return;
        }
        if let Some(i) = self.files.marked.iter().position(|p| *p == row.path) {
            self.files.marked.remove(i);
        } else {
            self.files.marked.push(row.path.clone());
        }
        self.message = format!(
            "{} marcados: a los copia al proyecto · m desmarca",
            self.files.marked.len()
        );
        self.files_step(1);
    }

    /// `path` está dentro del proyecto (la carpeta del script).
    fn in_project(&self, path: &Path) -> bool {
        crate::target::full_path(path).starts_with(crate::target::full_path(&self.script_dir()))
    }

    /// Lo que se añade: lo marcado o, si no hay, la fila elegida.
    fn chosen_for_adding(&self) -> Vec<PathBuf> {
        if !self.files.marked.is_empty() {
            return self.files.marked.clone();
        }
        self.files
            .tree
            .as_ref()
            .and_then(Tree::selected_row)
            .filter(|r| !r.note)
            .map(|r| vec![r.path])
            .unwrap_or_default()
    }

    /// «Añadir sample al proyecto» (`a`, o soltar en el editor lo
    /// arrastrado del árbol): lo de fuera del proyecto se copia a
    /// `samples/` y, si es un solo sample, se escribe su `sample("...")` en
    /// el cursor; lo que ya está dentro, solo se escribe.
    pub(super) fn add_to_project(&mut self) {
        let chosen = self.chosen_for_adding();
        self.add_paths(chosen);
    }

    pub(super) fn add_paths(&mut self, chosen: Vec<PathBuf>) {
        if chosen.is_empty() {
            self.message =
                "elige un sample o una carpeta en el explorador (Alt-0), o márcalos con m".into();
            return;
        }
        if chosen.len() == 1 && !chosen[0].is_dir() {
            let path = &chosen[0];
            match Kind::of(path, false) {
                Some(Kind::Audio) => {}
                Some(Kind::Unsupported) => {
                    self.message = unsupported_message(path);
                    return;
                }
                _ => {
                    self.message = "a: elige un sample (o una carpeta con samples)".into();
                    return;
                }
            }
            if self.in_project(path) {
                self.insert_reference(path);
                return;
            }
        }
        let outside: Vec<PathBuf> = chosen
            .iter()
            .filter(|p| !self.in_project(p))
            .cloned()
            .collect();
        if outside.is_empty() {
            self.message = "ya está en el proyecto: no hace falta copiarlo".into();
            return;
        }
        let single = chosen.len() == 1 && !chosen[0].is_dir();
        self.start_adding(&outside, single, Vec::new());
        self.files.marked.clear();
    }

    /// Empieza a copiar `paths` a `samples/` (en un hilo aparte).
    fn start_adding(&mut self, paths: &[PathBuf], insert: bool, gather: Vec<(String, PathBuf)>) {
        if self.files.adding.is_some() {
            self.message = "ya se están copiando samples: espera a que acabe".into();
            return;
        }
        let (copies, skipped) = crate::add_samples::plan(paths);
        if copies.is_empty() {
            self.message = if skipped.is_empty() {
                "no hay samples que copiar".into()
            } else {
                format!("nada que copiar: {}", skipped.join(" · "))
            };
            return;
        }
        let dest = self.project_samples();
        let files = copies.len();
        let running = crate::add_samples::start(dest, copies, self.sample_rate);
        self.files.adding = Some(Adding {
            running,
            insert,
            gather,
            skipped,
            shown: (u64::MAX, usize::MAX),
        });
        self.message = format!("copiando {files} al proyecto…");
    }

    /// Mira cómo va la copia (en cada vuelta): la barra en el mensaje y,
    /// al acabar, lo que se hizo.
    pub(super) fn poll_adding(&mut self) {
        let Some(adding) = &mut self.files.adding else {
            return;
        };
        let Some(outcomes) = adding.running.finished() else {
            let percent = (adding.running.fraction() * 100.0) as u64;
            let done = adding
                .running
                .done_files
                .load(std::sync::atomic::Ordering::Relaxed) as usize;
            if adding.shown != (percent, done) {
                adding.shown = (percent, done);
                self.message = format!(
                    "copiando al proyecto {} {percent}% · {done}/{}",
                    progress_bar(adding.running.fraction(), 20, crate::theme::ascii()),
                    adding.running.files
                );
            }
            return;
        };
        let Some(adding) = self.files.adding.take() else {
            return;
        };
        self.finish_adding(adding, outcomes);
    }

    fn finish_adding(&mut self, adding: Adding, outcomes: Vec<crate::add_samples::Outcome>) {
        use crate::add_samples::Outcome;
        let dest = self.project_samples();
        // `samples/` puede ser nueva: las raíces del árbol cambian.
        self.refresh_files();
        let placed: Vec<(PathBuf, PathBuf)> = outcomes
            .iter()
            .filter_map(Outcome::placed)
            .map(|(from, to)| (from.to_path_buf(), dest.join(to)))
            .collect();
        let failed: Vec<String> = outcomes
            .iter()
            .filter(|o| matches!(o, Outcome::Failed { .. }))
            .map(Outcome::describe)
            .chain(adding.skipped.iter().cloned())
            .collect();
        let mut notes: Vec<String> = outcomes
            .iter()
            .filter(|o| !matches!(o, Outcome::Failed { .. }))
            .map(Outcome::describe)
            .collect();
        if !adding.gather.is_empty() {
            notes.insert(0, self.rewrite_gathered(&adding.gather, &placed));
        } else if adding.insert
            && let [(_, full)] = placed.as_slice()
        {
            let full = full.clone();
            self.insert_reference(&full);
            notes.push(std::mem::take(&mut self.message));
        } else if !placed.is_empty() {
            notes.insert(0, format!("{} en samples/ del proyecto", placed.len()));
        }
        let mut message = notes.join(" · ");
        if !failed.is_empty() {
            message = format!("{message} · sin copiar: {}", failed.join(" · "));
        }
        self.message = message.trim_start_matches(" · ").to_string();
    }

    /// El texto más corto de `sample("...")` que encuentra `full` desde el
    /// script (ver `script::sample_path`): su ruta respecto a la primera
    /// carpeta donde se busca que lo tenga y en la que no lo tape otro; si
    /// no hay, la ruta entera.
    fn sample_name_for(&self, full: &Path) -> String {
        let full = crate::target::full_path(full);
        let dirs = self.host.sample_dirs(&self.script_dir());
        let mut best: Option<String> = None;
        for dir in dirs.order() {
            let dir = crate::target::full_path(&dir);
            let Ok(rel) = full.strip_prefix(&dir) else {
                continue;
            };
            let name = relative(rel, Path::new(""));
            if let script::sample_path::Resolved::Found { path, .. } = dirs.resolve(&name)
                && crate::target::full_path(&path) == full
                && best.as_ref().is_none_or(|b| name.len() < b.len())
            {
                best = Some(name);
            }
        }
        if let Some(name) = best {
            return name;
        }
        // Tapado por otro con el mismo nombre: con su carpeta delante.
        if let Ok(rel) = full.strip_prefix(crate::target::full_path(&self.script_dir())) {
            return relative(rel, Path::new(""));
        }
        full.display().to_string()
    }

    /// Escribe `sample("...")` de `path` en el cursor del editor (un paso de
    /// deshacer), con el aviso si el proyecto deja de poder moverse.
    fn insert_reference(&mut self, path: &Path) {
        let name = self.sample_name_for(path);
        if self.is_main_window() {
            self.message = format!("escribe sample(\"{name}\") en el editor de su ventana");
            return;
        }
        let text = format!("sample(\"{name}\")");
        self.marks.clear();
        self.grouped(|editor| editor.insert_str(&text));
        let full = crate::target::full_path(path);
        self.message = if self.in_project(&full) {
            format!("{text} en el script")
        } else if self
            .files
            .global
            .as_ref()
            .is_some_and(|g| full.starts_with(crate::target::full_path(g)))
        {
            format!(
                "{text} en el script: está en la carpeta global, no en el proyecto (en otra \
                 máquina hace falta la misma; a lo copia dentro)"
            )
        } else {
            format!(
                "{text} en el script: con su ruta entera, el proyecto deja de poder moverse \
                 (a lo copia dentro)"
            )
        };
    }

    /// «Referenciar sample sin copiar» (`A`): escribe `sample("...")` de la
    /// fila elegida con la ruta que lo encuentra, sin copiarlo.
    pub(super) fn reference_sample(&mut self) {
        let Some(row) = self.files.tree.as_ref().and_then(Tree::selected_row) else {
            self.message = "elige un sample en el explorador (Alt-0)".into();
            return;
        };
        match row.kind {
            Kind::Audio if !row.note => self.insert_reference(&row.path),
            Kind::Unsupported => self.message = unsupported_message(&row.path),
            _ => self.message = "A: elige un sample".into(),
        }
    }

    /// «Recoger los samples en el proyecto»: los `sample("...")` del código
    /// cuyo archivo está fuera del proyecto se copian a `samples/` y se
    /// reescribe su texto (un paso de deshacer). Para proyectos de antes.
    pub(super) fn gather_samples(&mut self) {
        if self.is_main_window() {
            self.message = "el editor está en su ventana: recógelos desde allí".into();
            return;
        }
        let text = self.text();
        let (literals, generated) = crate::add_samples::sample_literals(&text);
        let dirs = self.host.sample_dirs(&self.script_dir());
        let mut gather: Vec<(String, PathBuf)> = Vec::new();
        let mut missing = Vec::new();
        for (name, _) in &literals {
            if gather.iter().any(|(n, _)| n == name) {
                continue;
            }
            match dirs.resolve(name) {
                script::sample_path::Resolved::Found { path, .. } => {
                    if !self.in_project(&path) {
                        gather.push((name.clone(), path));
                    }
                }
                script::sample_path::Resolved::Missing { .. } => missing.push(name.clone()),
            }
        }
        let mut notes = Vec::new();
        if generated > 0 {
            notes.push(format!(
                "{generated} sample(...) sin un nombre escrito (generados por código): no se recogen"
            ));
        }
        if !missing.is_empty() {
            notes.push(format!("no están: {}", missing.join(", ")));
        }
        if gather.is_empty() {
            notes.insert(
                0,
                "todos los samples del script ya están en el proyecto".into(),
            );
            self.message = notes.join(" · ");
            return;
        }
        let paths: Vec<PathBuf> = gather.iter().map(|(_, p)| p.clone()).collect();
        self.start_adding(&paths, false, gather);
        if !notes.is_empty() {
            self.message = format!("{} · {}", self.message, notes.join(" · "));
        }
    }

    /// Al acabar de recoger: cada `sample("viejo")` pasa a la ruta de su
    /// copia, todo en un paso de deshacer, y se carga.
    fn rewrite_gathered(
        &mut self,
        gather: &[(String, PathBuf)],
        placed: &[(PathBuf, PathBuf)],
    ) -> String {
        let mut renames: Vec<(String, String)> = Vec::new();
        for (old, from) in gather {
            if let Some((_, full)) = placed.iter().find(|(f, _)| f == from) {
                let new = self.sample_name_for(full);
                if new != *old {
                    renames.push((old.clone(), new));
                }
            }
        }
        if renames.is_empty() {
            return format!("{} recogidos", placed.len());
        }
        let text = self.text();
        let (literals, _) = crate::add_samples::sample_literals(&text);
        let mut out = text.clone();
        // De atrás adelante, para que los sitios sigan valiendo.
        for (name, range) in literals.iter().rev() {
            if let Some((_, new)) = renames.iter().find(|(old, _)| old == name) {
                out.replace_range(range.clone(), new);
            }
        }
        if out != text {
            self.replace_text(&out);
            self.load();
        }
        format!(
            "{} recogidos en samples/ y su ruta cambiada en el código (Ctrl-Z lo deshace): {}",
            placed.len(),
            renames
                .iter()
                .map(|(a, b)| format!("{a} → {b}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }

    /// Una tecla mientras se escribe un nombre o se confirma un borrado.
    fn files_ask_key(&mut self, input: &Input) {
        let Some((ask, text)) = &mut self.files.ask else {
            return;
        };
        if let Ask::Delete(path) = ask {
            let path = path.clone();
            self.files.ask = None;
            let yes = matches!(input.key, Key::Enter | Key::Char('s' | 'S' | 'y' | 'Y'));
            if !yes {
                self.message = "no se borra".into();
                return;
            }
            self.message = match std::fs::remove_file(&path) {
                Ok(()) => format!("{} borrado", path.display()),
                Err(e) => format!("no se pudo borrar {}: {e}", path.display()),
            };
            if let Some(tree) = &mut self.files.tree {
                tree.refresh(None);
            }
            return;
        }
        match input.key {
            Key::Esc => {
                self.files.ask = None;
                self.message = "cancelado".into();
            }
            Key::Backspace => {
                text.pop();
            }
            Key::Char(c) if !input.ctrl && !input.alt => text.push(c),
            Key::Enter => {
                let (ask, text) = self.files.ask.take().unwrap();
                let name = match files::new_name(&text) {
                    Ok(name) => name,
                    Err(e) => {
                        self.message = e;
                        return;
                    }
                };
                let made = match &ask {
                    Ask::New(dir) => {
                        let path = dir.join(&name);
                        std::fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&path)
                            .map(|_| path)
                    }
                    Ask::Rename(from) => {
                        let path = from.parent().unwrap_or(Path::new(".")).join(&name);
                        if path.exists() {
                            Err(std::io::Error::new(
                                std::io::ErrorKind::AlreadyExists,
                                "ya existe",
                            ))
                        } else {
                            std::fs::rename(from, &path).map(|_| path)
                        }
                    }
                    Ask::Delete(_) => unreachable!(),
                };
                match made {
                    Ok(path) => {
                        if let Ask::Rename(from) = &ask
                            && self.is_current_script(from)
                        {
                            self.path = path.clone();
                        }
                        self.message = format!("{} listo", path.display());
                        if let Some(tree) = &mut self.files.tree {
                            tree.refresh(Some(&path));
                        }
                    }
                    Err(e) => self.message = format!("{name}: {e}"),
                }
            }
            _ => {}
        }
    }

    /// El ratón en el explorador: `true` si era suyo. Un clic elige la
    /// fila (y da el foco), en el icono de una carpeta la abre o cierra,
    /// dos clics la abren; la rueda recorre; arrastrar el borde derecho
    /// cambia el ancho.
    pub(super) fn files_mouse(&mut self, action: MouseAction, x: u16, y: u16) -> bool {
        // Un clic nuevo empieza de cero (un arrastre a medias se olvida).
        if action == MouseAction::Down {
            self.files.dragging = None;
        }
        // Un sample (o carpeta) arrastrado del árbol: soltarlo en el editor
        // lo añade al proyecto; en otro sitio, nada.
        if let Some(path) = self.files.dragging.clone()
            && matches!(action, MouseAction::Drag | MouseAction::Up)
        {
            if action == MouseAction::Up {
                self.files.dragging = None;
                let over_editor = self
                    .editor_area
                    .is_some_and(|area| area.contains((x, y).into()));
                if over_editor {
                    self.add_paths(vec![path]);
                } else if !self
                    .files
                    .layout
                    .is_some_and(|l| l.area.contains((x, y).into()))
                {
                    self.message = "suéltalo en el editor para añadirlo al proyecto".into();
                }
            } else if self
                .files
                .layout
                .is_some_and(|l| !l.area.contains((x, y).into()))
            {
                self.message = format!(
                    "arrastrando {}: suéltalo en el editor para añadirlo al proyecto",
                    path.file_name().unwrap_or_default().to_string_lossy()
                );
            }
            return true;
        }
        if self.files.drag {
            match action {
                MouseAction::Drag => {
                    if let Some(layout) = self.files.layout {
                        let wanted = x.saturating_sub(layout.area.x) + 1;
                        let body = self
                            .grid
                            .as_ref()
                            .and_then(|g| g.body)
                            .map_or(120, |b| b.width);
                        self.files.width = crate::layout::explorer_width(body, wanted);
                    }
                }
                MouseAction::Up => {
                    self.files.drag = false;
                    self.persist_layout();
                }
                _ => self.files.drag = false,
            }
            return true;
        }
        let Some(layout) = self.files.layout.filter(|_| self.files_visible()) else {
            return false;
        };
        if self
            .grid
            .as_ref()
            .is_some_and(|g| g.covered(Panel::Files, x, y))
        {
            return false;
        }
        let view = self.files_view();
        let Some(hit) = layout.hit_in(&view, x, y) else {
            // Dentro del panel pero en nada (el borde, la vista previa):
            // suyo igualmente.
            return layout.area.contains((x, y).into());
        };
        match (action, hit) {
            (MouseAction::Down, FilesHit::Edge) => self.files.drag = true,
            (MouseAction::Down, FilesHit::Arrow(i)) => {
                self.chosen_focus = Some(Panel::Files);
                if let Some(tree) = &mut self.files.tree {
                    tree.select(i);
                    if tree.can_toggle(i) {
                        tree.toggle(i);
                    }
                }
            }
            (MouseAction::Down, FilesHit::Row(i)) => {
                self.chosen_focus = Some(Panel::Files);
                let now = Instant::now();
                let double = self
                    .files
                    .last_click
                    .is_some_and(|(t, row)| row == i && now.duration_since(t) < DOUBLE_CLICK);
                self.files.last_click = Some((now, i));
                if let Some(tree) = &mut self.files.tree {
                    tree.select(i);
                }
                // Puede empezar a arrastrarse (un sample o una carpeta).
                self.files.dragging = self
                    .files
                    .tree
                    .as_ref()
                    .and_then(Tree::selected_row)
                    .filter(|r| !r.note && matches!(r.kind, Kind::Audio | Kind::Dir))
                    .map(|r| r.path);
                if double {
                    self.files.last_click = None;
                    self.activate_file();
                } else {
                    self.message = files_keys().into();
                }
            }
            (MouseAction::ScrollUp, _) => self.files_step(-3),
            (MouseAction::ScrollDown, _) => self.files_step(3),
            _ => {}
        }
        true
    }
}

/// Una barra de progreso de `width` celdas.
fn progress_bar(fraction: f64, width: usize, ascii: bool) -> String {
    let full = ((fraction.clamp(0.0, 1.0) * width as f64).round()) as usize;
    let (on, off, open, close) = if ascii {
        ("#", ".", "[", "]")
    } else {
        ("█", "░", "▕", "▏")
    };
    format!(
        "{open}{}{}{close}",
        on.repeat(full),
        off.repeat(width - full)
    )
}

/// Lo que se dice de un archivo de audio que el motor no carga.
fn unsupported_message(path: &Path) -> String {
    let ext = path
        .extension()
        .map_or_else(String::new, |e| e.to_string_lossy().to_lowercase());
    format!(
        "{}: formato no soportado: {ext} (se cargan {}); conviértelo, por ejemplo con ffmpeg -i \
         entrada salida.wav",
        path.file_name().unwrap_or_default().to_string_lossy(),
        files::AUDIO.join(", ")
    )
}

/// Las teclas del explorador, para el mensaje.
pub fn files_keys() -> &'static str {
    "ARCHIVOS: ↑↓ elegir · Enter abre · ←→ plegar · Espacio escucha · a añadir al proyecto · A solo referenciar · m marcar · n nuevo · r renombra · Supr borra · R refresca · </> ancho · Esc vuelve"
}

/// `path` respecto a `base` (con `/`), o entera si no está dentro.
fn relative(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .map(|p| {
            p.components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_else(|_| path.display().to_string())
}
