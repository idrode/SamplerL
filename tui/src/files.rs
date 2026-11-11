//! El árbol del explorador de archivos, sin terminal: las carpetas desde
//! la del proyecto, con los scripts (`.lua`), los proyectos (`.sampler`)
//! y los samples. Cada carpeta se lee al abrirla por primera vez (y al
//! refrescar), nunca antes, y de una carpeta enorme solo las primeras
//! [`MAX_ENTRIES`]: así abrir el árbol no recorre el disco y un
//! directorio gigante no para el dibujo. Solo lo usa el hilo de la
//! interfaz.

use std::path::{Path, PathBuf};

/// Entradas que se leen como mucho de una carpeta.
pub const MAX_ENTRIES: usize = 500;

/// Qué es cada entrada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Dir,
    Script,
    Project,
    Audio,
    /// Audio que el motor no sabe cargar: se ve atenuado, con su nota.
    Unsupported,
}

/// Las extensiones de los samples que se pueden cargar (`sample()` lee
/// WAV, FLAC, MP3, OGG y AIFF, como el explorador de samples).
pub const AUDIO: [&str; engine::SAMPLE_EXTENSIONS.len()] = engine::SAMPLE_EXTENSIONS;

/// Extensiones de audio que el motor no sabe cargar: se enseñan atenuadas
/// con «formato no soportado», nunca se esconden sin decirlo.
pub const UNSUPPORTED_AUDIO: [&str; 17] = [
    "m4a", "aac", "opus", "wma", "alac", "ape", "wv", "caf", "mp2", "mpc", "ac3", "amr", "au",
    "snd", "w64", "rf64", "mka",
];

impl Kind {
    /// Qué es `path` (`None`: no se enseña).
    pub fn of(path: &Path, is_dir: bool) -> Option<Kind> {
        if is_dir {
            return Some(Kind::Dir);
        }
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "lua" => Kind::Script,
            "sampler" => Kind::Project,
            e if AUDIO.contains(&e) => Kind::Audio,
            e if UNSUPPORTED_AUDIO.contains(&e) => Kind::Unsupported,
            _ => return None,
        })
    }

    /// Su icono (con `ascii`, sin Unicode); el de una carpeta, abierta o
    /// cerrada.
    pub fn icon(self, open: bool, ascii: bool) -> &'static str {
        match (self, ascii) {
            (Kind::Dir, false) if open => "▾",
            (Kind::Dir, false) => "▸",
            (Kind::Dir, true) if open => "v",
            (Kind::Dir, true) => ">",
            (Kind::Script, false) => "λ",
            (Kind::Script, true) => "s",
            (Kind::Project, false) => "◆",
            (Kind::Project, true) => "p",
            (Kind::Audio, false) => "♪",
            (Kind::Audio, true) => "~",
            (Kind::Unsupported, false) => "♪",
            (Kind::Unsupported, true) => "~",
        }
    }
}

/// Una entrada del árbol.
#[derive(Debug, Clone)]
struct Entry {
    name: String,
    path: PathBuf,
    kind: Kind,
    /// De una carpeta: lo leído (`None`: aún no) y si está abierta.
    children: Option<Vec<Entry>>,
    open: bool,
    /// Las que no se leyeron por pasar de [`MAX_ENTRIES`].
    more: usize,
    /// No se pudo leer (el error).
    error: Option<String>,
    /// Archivos de la carpeta que no se enseñan (ni audio ni scripts).
    hidden: usize,
    /// Un enlace a una carpeta que la contiene (o a sí misma): no se
    /// sigue, para no dar vueltas sin fin.
    cycle: bool,
    /// De una raíz: cuál es.
    label: Option<String>,
}

impl Entry {
    fn new(path: PathBuf, kind: Kind) -> Entry {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into(),
        );
        Entry {
            name,
            path,
            kind,
            children: None,
            open: false,
            more: 0,
            error: None,
            hidden: 0,
            cycle: false,
            label: None,
        }
    }

    /// Lee la carpeta (las primeras [`MAX_ENTRIES`], sin las ocultas):
    /// primero las carpetas y luego los archivos, por nombre.
    fn read(&mut self) {
        let mut out = Vec::new();
        self.more = 0;
        self.error = None;
        self.hidden = 0;
        if self.cycle {
            self.children = Some(out);
            return;
        }
        // La carpeta de verdad, para reconocer los enlaces que vuelven a
        // ella o a una de las de arriba.
        let real = std::fs::canonicalize(&self.path).ok();
        match std::fs::read_dir(&self.path) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    if name.to_string_lossy().starts_with('.') {
                        continue;
                    }
                    let path = entry.path();
                    // El tipo sin seguir el enlace (barato); un enlace, mirando
                    // a dónde lleva.
                    let is_dir = match entry.file_type() {
                        Ok(t) if t.is_symlink() => path.is_dir(),
                        Ok(t) => t.is_dir(),
                        Err(_) => false,
                    };
                    let Some(kind) = Kind::of(&path, is_dir) else {
                        self.hidden += 1;
                        continue;
                    };
                    let cycle = is_dir
                        && entry.file_type().is_ok_and(|t| t.is_symlink())
                        && match (std::fs::canonicalize(&path), &real) {
                            (Ok(target), Some(real)) => real.starts_with(target),
                            _ => false,
                        };
                    if out.len() >= MAX_ENTRIES {
                        self.more += 1;
                        // Contar sin fin tampoco: se para en el doble.
                        if self.more >= MAX_ENTRIES {
                            break;
                        }
                        continue;
                    }
                    let mut child = Entry::new(path, kind);
                    child.cycle = cycle;
                    out.push(child);
                }
            }
            Err(e) => {
                let full = crate::target::full_path(&self.path);
                self.error = Some(if e.kind() == std::io::ErrorKind::NotFound {
                    format!("no existe: {}", full.display())
                } else {
                    format!("no se puede leer {}: {e}", full.display())
                });
            }
        }
        out.sort_by(|a, b| {
            (a.kind != Kind::Dir)
                .cmp(&(b.kind != Kind::Dir))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        self.children = Some(out);
    }

    /// Vuelve a leer las carpetas abiertas (conserva lo abierto dentro).
    fn refresh(&mut self) {
        if self.kind != Kind::Dir || self.children.is_none() {
            return;
        }
        let old = self.children.take().unwrap_or_default();
        self.read();
        for child in self.children.iter_mut().flatten() {
            if let Some(was) = old.iter().find(|o| o.path == child.path)
                && was.open
            {
                child.open = true;
                child.children = was.children.clone();
                child.refresh();
            }
        }
    }
}

/// Una fila del árbol tal como se ve.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub depth: usize,
    pub name: String,
    pub path: PathBuf,
    pub kind: Kind,
    pub open: bool,
    /// Una fila de aviso (`… 300 más`, el error al leer), no un archivo.
    pub note: bool,
    /// Lo que se dice detrás del nombre, atenuado (`formato no soportado:
    /// m4a`).
    pub hint: Option<String>,
    /// De una raíz: cuál es (`proyecto`, `global`, `script`).
    pub label: Option<String>,
    /// Dónde está en el árbol: la raíz y el hijo de cada carpeta (una
    /// misma carpeta puede estar bajo dos raíces).
    pub at: Vec<usize>,
}

/// Una raíz del árbol: qué es y su carpeta.
#[derive(Debug, Clone, PartialEq)]
pub struct Root {
    pub label: String,
    pub dir: PathBuf,
}

/// El árbol, con la fila elegida y la primera que se ve.
#[derive(Debug, Clone)]
pub struct Tree {
    roots: Vec<Entry>,
    pub selected: usize,
    pub top: usize,
}

impl Tree {
    /// El árbol de `dir`, con la raíz abierta (se lee solo esa carpeta).
    #[cfg(test)]
    pub fn new(dir: &Path) -> Tree {
        Tree::with_roots(&[Root {
            label: String::new(),
            dir: dir.to_path_buf(),
        }])
    }

    /// Un árbol con varias raíces, en ese orden, cada una abierta (se lee
    /// solo cada raíz) y con su nombre delante; la misma carpeta dos veces
    /// solo sale la primera.
    pub fn with_roots(roots: &[Root]) -> Tree {
        let mut entries: Vec<Entry> = Vec::new();
        for root in roots {
            let dir = crate::target::full_path(&root.dir);
            if entries.iter().any(|e| e.path == dir) {
                continue;
            }
            let mut entry = Entry::new(dir, Kind::Dir);
            entry.label = (!root.label.is_empty()).then(|| root.label.clone());
            entry.open = true;
            entry.read();
            entries.push(entry);
        }
        Tree {
            roots: entries,
            selected: 0,
            top: 0,
        }
    }

    /// Las carpetas de las raíces, en orden.
    pub fn root_dirs(&self) -> Vec<PathBuf> {
        self.roots.iter().map(|r| r.path.clone()).collect()
    }

    /// Las filas que se ven, de arriba abajo.
    pub fn rows(&self) -> Vec<Row> {
        let mut out = Vec::new();
        for (i, root) in self.roots.iter().enumerate() {
            flatten(root, 0, &mut vec![i], &mut out);
        }
        out
    }

    /// La fila elegida.
    pub fn selected_row(&self) -> Option<Row> {
        self.rows().into_iter().nth(self.selected)
    }

    /// Elige la fila `i` (dentro de las que hay).
    pub fn select(&mut self, i: usize) {
        let n = self.rows().len();
        self.selected = i.min(n.saturating_sub(1));
    }

    /// Mueve la elegida `delta` filas.
    pub fn step(&mut self, delta: isize) {
        let n = self.rows().len() as isize;
        if n == 0 {
            return;
        }
        self.selected = (self.selected as isize + delta).clamp(0, n - 1) as usize;
    }

    /// La fila `i` se puede plegar: una carpeta que no es la única raíz.
    pub fn can_toggle(&self, i: usize) -> bool {
        self.rows().get(i).is_some_and(|r| {
            r.kind == Kind::Dir && !r.note && (r.depth > 0 || self.roots.len() > 1)
        })
    }

    /// Abre o cierra la carpeta de la fila `i` (la lee la primera vez).
    pub fn toggle(&mut self, i: usize) {
        if let Some(entry) = self.entry_at(i) {
            set_open(entry, !entry.open);
        }
    }

    /// Abre la carpeta de la fila `i` (`false` si no es una carpeta o ya
    /// estaba abierta).
    pub fn open(&mut self, i: usize) -> bool {
        match self.entry_at(i) {
            Some(entry) if !entry.open => {
                set_open(entry, true);
                true
            }
            _ => false,
        }
    }

    /// ←: cierra la carpeta de la elegida si está abierta; si no, va a la
    /// carpeta que la contiene.
    pub fn close_or_parent(&mut self) {
        let rows = self.rows();
        let Some(row) = rows.get(self.selected) else {
            return;
        };
        if row.open && self.can_toggle(self.selected) {
            let i = self.selected;
            self.toggle(i);
            return;
        }
        let depth = row.depth;
        if let Some(parent) = rows[..self.selected]
            .iter()
            .rposition(|r| r.depth + 1 == depth && !r.note)
        {
            self.selected = parent;
        }
    }

    /// Vuelve a leer lo abierto (tras crear, renombrar o borrar, o a mano)
    /// y elige `path` si está.
    pub fn refresh(&mut self, select: Option<&Path>) {
        for root in &mut self.roots {
            root.refresh();
        }
        if let Some(path) = select {
            self.reveal(path);
        }
        let n = self.rows().len();
        self.selected = self.selected.min(n.saturating_sub(1));
    }

    /// Abre las carpetas hasta `path` (en la primera raíz que lo tenga) y
    /// lo elige.
    pub fn reveal(&mut self, path: &Path) {
        let path = crate::target::full_path(path);
        let Some(r) = self.roots.iter().position(|e| path.starts_with(&e.path)) else {
            return;
        };
        let mut at = vec![r];
        let mut entry = &mut self.roots[r];
        let rel = path
            .strip_prefix(&entry.path)
            .unwrap_or(Path::new(""))
            .to_path_buf();
        for part in rel.parent().into_iter().flat_map(|p| p.components()) {
            let next = entry.path.join(part);
            set_open(entry, true);
            let Some(i) = entry
                .children
                .as_ref()
                .and_then(|c| c.iter().position(|e| e.path == next))
            else {
                return;
            };
            at.push(i);
            let Some(child) = entry.children.as_mut().and_then(|c| c.get_mut(i)) else {
                return;
            };
            entry = child;
        }
        set_open(entry, true);
        if let Some(i) = self
            .rows()
            .iter()
            .position(|row| row.path == path && !row.note && row.at.starts_with(&at))
        {
            self.selected = i;
        }
    }

    /// Deja `top` de modo que la elegida se vea en `height` filas.
    pub fn scroll_to(&mut self, height: usize) {
        if height == 0 {
            return;
        }
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + height {
            self.top = self.selected + 1 - height;
        }
        let n = self.rows().len();
        self.top = self.top.min(n.saturating_sub(height));
    }

    /// La entrada de la carpeta de la fila `i`.
    fn entry_at(&mut self, i: usize) -> Option<&mut Entry> {
        let at = self.rows().get(i).filter(|r| !r.note)?.at.clone();
        let (first, rest) = at.split_first()?;
        let mut entry = self.roots.get_mut(*first)?;
        for &i in rest {
            entry = entry.children.as_mut()?.get_mut(i)?;
        }
        (entry.kind == Kind::Dir).then_some(entry)
    }
}

fn set_open(entry: &mut Entry, open: bool) {
    if entry.kind != Kind::Dir {
        return;
    }
    if open && entry.children.is_none() {
        entry.read();
    }
    entry.open = open;
}

fn flatten(entry: &Entry, depth: usize, at: &mut Vec<usize>, out: &mut Vec<Row>) {
    let hint = match entry.kind {
        Kind::Unsupported => Some(format!(
            "formato no soportado: {}",
            entry
                .path
                .extension()
                .map_or_else(String::new, |e| e.to_string_lossy().to_lowercase())
        )),
        Kind::Dir if entry.cycle => Some("enlace a una carpeta de arriba: no se sigue".into()),
        _ => None,
    };
    out.push(Row {
        depth,
        name: entry.name.clone(),
        path: entry.path.clone(),
        kind: entry.kind,
        open: entry.open,
        note: false,
        hint,
        label: entry.label.clone(),
        at: at.clone(),
    });
    if !entry.open {
        return;
    }
    let here = at.clone();
    let note = |name: String| Row {
        depth: depth + 1,
        name,
        path: entry.path.clone(),
        kind: Kind::Dir,
        open: false,
        note: true,
        hint: None,
        label: None,
        at: here.clone(),
    };
    if let Some(error) = &entry.error {
        out.push(note(error.clone()));
    }
    for (i, child) in entry.children.iter().flatten().enumerate() {
        at.push(i);
        flatten(child, depth + 1, at, out);
        at.pop();
    }
    if entry.more > 0 {
        let more = if entry.more >= MAX_ENTRIES {
            format!("… {} o más sin leer", entry.more)
        } else {
            format!("… {} más sin leer", entry.more)
        };
        out.push(note(more));
    }
    if entry.hidden > 0 {
        let what = if entry.hidden == 1 {
            "1 archivo más que no es audio ni script".to_string()
        } else {
            format!("{} archivos más que no son audio ni scripts", entry.hidden)
        };
        out.push(note(format!("({what})")));
    }
    if entry.children.as_ref().is_some_and(|c| c.is_empty())
        && entry.error.is_none()
        && entry.hidden == 0
        && !entry.cycle
    {
        out.push(note(format!(
            "(vacía: {})",
            crate::target::full_path(&entry.path).display()
        )));
    }
}

/// Un nombre de archivo nuevo válido (sin barras ni `..`), con `.lua` si
/// no lleva extensión.
pub fn new_name(text: &str) -> Result<String, String> {
    let name = text.trim();
    if name.is_empty() {
        return Err("falta el nombre".into());
    }
    if name.contains('/') || name.contains('\\') || name == "." || name == ".." {
        return Err(format!("«{name}»: solo un nombre, sin carpetas"));
    }
    if name.starts_with('.') {
        return Err(format!("«{name}»: los ocultos no se ven en el árbol"));
    }
    Ok(if Path::new(name).extension().is_none() {
        format!("{name}.lua")
    } else {
        name.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(test: &str) -> PathBuf {
        let dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../target/tmp"))
            .join(format!("files_{test}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        std::fs::create_dir_all(dir.join("samples/breaks")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        for f in [
            "song.lua",
            "p.sampler",
            "notas.txt",
            "scripts/01-intro.lua",
            "samples/amen.wav",
            "samples/Think.WAV",
            "samples/notas.txt",
            "samples/breaks/apache.wav",
        ] {
            std::fs::write(dir.join(f), "").unwrap();
        }
        dir
    }

    fn names(tree: &Tree) -> Vec<String> {
        tree.rows()
            .iter()
            .map(|r| format!("{}{}", "  ".repeat(r.depth), r.name))
            .collect()
    }

    #[test]
    fn only_the_root_is_read_and_folders_open_on_demand() {
        let dir = dir("lazy");
        let mut tree = Tree::new(&dir);
        assert_eq!(
            names(&tree),
            [
                "files_lazy",
                "  samples",
                "  scripts",
                "  p.sampler",
                "  song.lua",
                "  (1 archivo más que no es audio ni script)"
            ]
        );
        // Nada leído aún dentro de las carpetas cerradas.
        assert!(
            tree.roots[0].children.as_ref().unwrap()[0]
                .children
                .is_none()
        );
        tree.toggle(1);
        assert_eq!(
            names(&tree)[1..5],
            ["  samples", "    breaks", "    amen.wav", "    Think.WAV"]
        );
        tree.toggle(1);
        assert_eq!(names(&tree).len(), 6);
        // Reabierta, ya leída.
        tree.open(1);
        assert_eq!(names(&tree).len(), 10);
    }

    #[test]
    fn left_closes_or_goes_up_and_reveal_opens_the_way() {
        let dir = dir("nav");
        let mut tree = Tree::new(&dir);
        tree.reveal(&dir.join("samples/breaks/apache.wav"));
        let row = tree.selected_row().unwrap();
        assert_eq!(row.name, "apache.wav");
        assert_eq!(row.depth, 3);
        tree.close_or_parent();
        assert_eq!(tree.selected_row().unwrap().name, "breaks");
        tree.close_or_parent();
        assert!(!tree.selected_row().unwrap().open);
        tree.close_or_parent();
        assert_eq!(tree.selected_row().unwrap().name, "samples");
    }

    #[test]
    fn a_huge_folder_reads_only_the_first_entries() {
        let dir = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../target/tmp/files_huge"
        ));
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir.join("muchos")).unwrap();
        for i in 0..MAX_ENTRIES + 20 {
            std::fs::write(dir.join(format!("muchos/{i:04}.wav")), "").unwrap();
        }
        let mut tree = Tree::new(dir);
        let start = std::time::Instant::now();
        tree.toggle(1);
        let rows = tree.rows();
        // Holgado: con la máquina cargada; lo que importa es que no lee
        // todas las entradas (serían segundos con miles).
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        assert_eq!(rows.len(), 2 + MAX_ENTRIES + 1);
        let last = rows.last().unwrap();
        assert!(last.note && last.name.contains("20 más"), "{last:?}");
    }

    #[test]
    fn refresh_keeps_what_is_open_and_finds_what_is_new() {
        let dir = dir("refresh");
        let mut tree = Tree::new(&dir);
        tree.toggle(2);
        std::fs::write(dir.join("scripts/02-drop.lua"), "").unwrap();
        tree.refresh(Some(&dir.join("scripts/02-drop.lua")));
        assert_eq!(tree.selected_row().unwrap().name, "02-drop.lua");
        assert!(names(&tree).contains(&"    01-intro.lua".to_string()));
    }

    #[test]
    fn a_link_back_up_is_not_followed_and_a_missing_root_says_so() {
        let dir = dir("cycle");
        std::os::unix::fs::symlink(&dir, dir.join("samples/vuelta")).unwrap();
        std::os::unix::fs::symlink(dir.join("scripts"), dir.join("samples/otra")).unwrap();
        let mut tree = Tree::new(&dir);
        tree.toggle(1);
        let rows = tree.rows();
        let back = rows.iter().find(|r| r.name == "vuelta").unwrap();
        assert_eq!(
            back.hint.as_deref(),
            Some("enlace a una carpeta de arriba: no se sigue")
        );
        let i = rows.iter().position(|r| r.name == "vuelta").unwrap();
        tree.toggle(i);
        assert_eq!(tree.rows().len(), rows.len(), "no se abre");
        // Un enlace a una carpeta de al lado sí se sigue.
        let i = rows.iter().position(|r| r.name == "otra").unwrap();
        tree.toggle(i);
        assert!(tree.rows().iter().any(|r| r.name == "01-intro.lua"));

        let missing = dir.join("no-esta");
        let tree = Tree::new(&missing);
        let rows = tree.rows();
        assert_eq!(
            rows[1].name,
            format!(
                "no existe: {}",
                missing
                    .canonicalize()
                    .unwrap_or(crate::target::full_path(&missing))
                    .display()
            )
        );
    }

    #[test]
    fn new_names_get_lua_and_cannot_escape() {
        assert_eq!(new_name("drop").unwrap(), "drop.lua");
        assert_eq!(new_name("bajo.sampler").unwrap(), "bajo.sampler");
        assert!(new_name("../x").is_err());
        assert!(new_name("  ").is_err());
        assert!(new_name(".oculto").is_err());
    }

    #[test]
    fn icons_degrade_to_ascii() {
        for kind in [Kind::Dir, Kind::Script, Kind::Project, Kind::Audio] {
            for open in [true, false] {
                assert!(kind.icon(open, true).is_ascii());
            }
        }
        assert_eq!(Kind::of(Path::new("a.WAV"), false), Some(Kind::Audio));
        assert_eq!(Kind::of(Path::new("a.flac"), false), Some(Kind::Audio));
        assert_eq!(Kind::of(Path::new("a.txt"), false), None);
    }
}
