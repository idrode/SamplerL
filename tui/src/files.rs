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
}

/// Las extensiones de los samples que se pueden cargar (`sample()` lee
/// WAV, como el explorador de samples).
pub const AUDIO: [&str; 1] = ["wav"];

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
        }
    }

    /// Lee la carpeta (las primeras [`MAX_ENTRIES`], sin las ocultas):
    /// primero las carpetas y luego los archivos, por nombre.
    fn read(&mut self) {
        let mut out = Vec::new();
        self.more = 0;
        self.error = None;
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
                        continue;
                    };
                    if out.len() >= MAX_ENTRIES {
                        self.more += 1;
                        // Contar sin fin tampoco: se para en el doble.
                        if self.more >= MAX_ENTRIES {
                            break;
                        }
                        continue;
                    }
                    out.push(Entry::new(path, kind));
                }
            }
            Err(e) => self.error = Some(e.to_string()),
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
}

/// El árbol, con la fila elegida y la primera que se ve.
#[derive(Debug, Clone)]
pub struct Tree {
    root: Entry,
    pub selected: usize,
    pub top: usize,
}

impl Tree {
    /// El árbol de `dir`, con la raíz abierta (se lee solo esa carpeta).
    pub fn new(dir: &Path) -> Tree {
        let mut root = Entry::new(dir.to_path_buf(), Kind::Dir);
        root.open = true;
        root.read();
        Tree {
            root,
            selected: 0,
            top: 0,
        }
    }

    /// Las filas que se ven, de arriba abajo.
    pub fn rows(&self) -> Vec<Row> {
        let mut out = Vec::new();
        flatten(&self.root, 0, &mut out);
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
        if row.kind == Kind::Dir && row.open && self.selected > 0 {
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
        self.root.refresh();
        if let Some(path) = select {
            self.reveal(path);
        }
        let n = self.rows().len();
        self.selected = self.selected.min(n.saturating_sub(1));
    }

    /// Abre las carpetas hasta `path` (si está dentro de la raíz) y lo
    /// elige.
    pub fn reveal(&mut self, path: &Path) {
        let Ok(rel) = path.strip_prefix(&self.root.path) else {
            return;
        };
        let mut entry = &mut self.root;
        for part in rel.parent().into_iter().flat_map(|p| p.components()) {
            let next = entry.path.join(part);
            set_open(entry, true);
            let Some(child) = entry
                .children
                .as_mut()
                .and_then(|c| c.iter_mut().find(|e| e.path == next))
            else {
                return;
            };
            entry = child;
        }
        set_open(entry, true);
        if let Some(i) = self.rows().iter().position(|r| r.path == path && !r.note) {
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
        let path = self.rows().get(i).filter(|r| !r.note)?.path.clone();
        find_mut(&mut self.root, &path).filter(|e| e.kind == Kind::Dir)
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

fn find_mut<'a>(entry: &'a mut Entry, path: &Path) -> Option<&'a mut Entry> {
    if entry.path == path {
        return Some(entry);
    }
    if !path.starts_with(&entry.path) {
        return None;
    }
    entry
        .children
        .as_mut()?
        .iter_mut()
        .find_map(|c| find_mut(c, path))
}

fn flatten(entry: &Entry, depth: usize, out: &mut Vec<Row>) {
    out.push(Row {
        depth,
        name: entry.name.clone(),
        path: entry.path.clone(),
        kind: entry.kind,
        open: entry.open,
        note: false,
    });
    if !entry.open {
        return;
    }
    let note = |name: String| Row {
        depth: depth + 1,
        name,
        path: entry.path.clone(),
        kind: Kind::Dir,
        open: false,
        note: true,
    };
    if let Some(error) = &entry.error {
        out.push(note(format!("no se puede leer: {error}")));
    }
    for child in entry.children.iter().flatten() {
        flatten(child, depth + 1, out);
    }
    if entry.more > 0 {
        let more = if entry.more >= MAX_ENTRIES {
            format!("… {} o más sin leer", entry.more)
        } else {
            format!("… {} más sin leer", entry.more)
        };
        out.push(note(more));
    }
    if entry.children.as_ref().is_some_and(|c| c.is_empty()) && entry.error.is_none() {
        out.push(note("(vacía)".into()));
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
            "samples/notas.flac",
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
                "  song.lua"
            ]
        );
        // Nada leído aún dentro de las carpetas cerradas.
        assert!(tree.root.children.as_ref().unwrap()[0].children.is_none());
        tree.toggle(1);
        assert_eq!(
            names(&tree)[1..5],
            ["  samples", "    breaks", "    amen.wav", "    Think.WAV"]
        );
        tree.toggle(1);
        assert_eq!(names(&tree).len(), 5);
        // Reabierta, ya leída.
        tree.open(1);
        assert_eq!(names(&tree).len(), 8);
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
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
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
        assert_eq!(Kind::of(Path::new("a.txt"), false), None);
        assert_eq!(Kind::of(Path::new("a.flac"), false), None);
    }
}
