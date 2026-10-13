//! Biblioteca de bloques: trozos de script reutilizables guardados en
//! archivos `.lua`, en una carpeta global (`~/.config/sampler/blocks/`) y
//! otra del proyecto (`blocks/` junto al proyecto o al script). Se insertan
//! como texto en el editor: nunca se cargan ni se enlazan desde un script,
//! así que un bloque con Lua no válido solo da error al cargar el script en
//! el que se pegó (y sigue sonando lo anterior).
//!
//! Formato: una cabecera de comentarios al principio y el cuerpo detrás.
//! Los `${...}` del cuerpo son campos que se rellenan saltando con Tab,
//! como en los snippets ([`crate::api::expand_template`]).
//!
//! ```text
//! -- nombre: ghost_suave
//! -- descripción: ghost notes en contratiempos
//! -- etiquetas: ghost, hats
//!
//! track("${ghost}", amen:slice_at(${3}))
//!   :steps(". x@0.2 . x@0.1  . x@0.2 . .  . x@0.2 . x@0.1  . . x@0.2 .")
//! ```
//!
//! Sin `nombre`, el bloque se llama como el archivo.

use std::path::{Path, PathBuf};

use crate::api;

/// Nombre de la carpeta de bloques (dentro de la del proyecto o de
/// `~/.config/sampler/`).
pub const DIR: &str = "blocks";

/// Claves de la cabecera (con sus alternativas al leer).
pub const KEY_NAME: &str = "nombre";
pub const KEY_DESCRIPTION: &str = "descripción";
pub const KEY_TAGS: &str = "etiquetas";

/// Dónde vive un bloque.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// En la carpeta global, para todos los proyectos.
    Global,
    /// En la carpeta `blocks/` del proyecto.
    Project,
}

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Scope::Global => "global",
            Scope::Project => "proyecto",
        }
    }
}

/// Un bloque de la biblioteca.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    /// Cuerpo, con los campos `${...}`; acaba en salto de línea.
    pub body: String,
    pub scope: Scope,
    /// Archivo del que se leyó (o donde se guardará).
    pub path: PathBuf,
}

impl Block {
    /// Texto del archivo: cabecera, línea en blanco y cuerpo.
    pub fn to_file(&self) -> String {
        file_text(&self.name, &self.description, &self.tags, &self.body)
    }

    /// El cuerpo tal como queda al insertarlo (sin las marcas de campo).
    pub fn preview(&self) -> String {
        api::expand_template(&self.body).0
    }

    /// Cada palabra de la búsqueda aparece en el nombre, las etiquetas o
    /// la descripción (sin distinguir mayúsculas ni tildes).
    pub fn matches(&self, query: &str) -> bool {
        let haystack = fold(&format!(
            "{} {} {}",
            self.name,
            self.tags.join(" "),
            self.description
        ));
        query
            .split_whitespace()
            .all(|w| haystack.contains(fold(w).trim_start_matches('#')))
    }
}

fn file_text(name: &str, description: &str, tags: &[String], body: &str) -> String {
    let mut text = format!("-- {KEY_NAME}: {name}\n");
    if !description.is_empty() {
        text.push_str(&format!("-- {KEY_DESCRIPTION}: {description}\n"));
    }
    if !tags.is_empty() {
        text.push_str(&format!("-- {KEY_TAGS}: {}\n", tags.join(", ")));
    }
    text.push('\n');
    text.push_str(&normalize_body(body));
    text
}

/// Cuerpo con un único salto de línea al final.
fn normalize_body(body: &str) -> String {
    let mut body = body.trim_end_matches(['\n', '\r']).to_string();
    body.push('\n');
    body
}

/// Minúsculas y sin tildes, para comparar.
pub fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'ä' => 'a',
            'é' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'ï' => 'i',
            'ó' | 'ò' | 'ö' => 'o',
            'ú' | 'ù' | 'ü' => 'u',
            'ñ' => 'n',
            c => c,
        })
        .collect()
}

/// Comprueba un nombre de bloque: letras, números y `_`, empezando por
/// letra o `_` (así se puede escribir en el autocompletado).
pub fn check_name(name: &str) -> Result<(), String> {
    let valid = name
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    if valid {
        Ok(())
    } else {
        Err(format!(
            "nombre de bloque no válido «{name}»: letras, números y _, empezando por letra"
        ))
    }
}

/// Etiquetas separadas por comas o espacios (sin `#` delante ni
/// repetidas).
pub fn parse_tags(text: &str) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for tag in text
        .split([',', ' ', '\t'])
        .map(|t| t.trim().trim_start_matches('#'))
    {
        if !tag.is_empty() && !tags.iter().any(|t| t == tag) {
            tags.push(tag.to_string());
        }
    }
    tags
}

/// Cabecera y cuerpo de un archivo de bloque: (nombre, descripción,
/// etiquetas, cuerpo). La cabecera son las líneas `-- clave: valor` del
/// principio con una clave conocida; lo demás es el cuerpo (la línea en
/// blanco que la separa no cuenta).
pub fn parse(text: &str) -> (Option<String>, String, Vec<String>, String) {
    let (mut name, mut description, mut tags) = (None, String::new(), Vec::new());
    let mut lines = text.lines().peekable();
    let mut header = false;
    while let Some(line) = lines.peek() {
        let Some((key, value)) = line
            .strip_prefix("--")
            .and_then(|rest| rest.split_once(':'))
        else {
            break;
        };
        let value = value.trim();
        match fold(key.trim()).as_str() {
            "nombre" | "name" => name = Some(value.to_string()),
            "descripcion" | "description" => description = value.to_string(),
            "etiquetas" | "tags" => tags = parse_tags(value),
            _ => break,
        }
        header = true;
        lines.next();
    }
    if header && lines.peek().is_some_and(|l| l.trim().is_empty()) {
        lines.next();
    }
    let body: Vec<&str> = lines.collect();
    let body = if body.is_empty() {
        String::new()
    } else {
        normalize_body(&body.join("\n"))
    };
    (name.filter(|n| !n.is_empty()), description, tags, body)
}

/// Carpeta global de bloques: `$XDG_CONFIG_HOME/sampler/blocks` o
/// `~/.config/sampler/blocks`. `None` si no hay ni una ni otra variable.
pub fn global_dir() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(config.join("sampler").join(DIR))
}

/// Carpeta de bloques de un proyecto (o de un script suelto) en `base`.
pub fn project_dir(base: &Path) -> PathBuf {
    base.join(DIR)
}

/// Error al guardar un bloque.
#[derive(Debug, Clone, PartialEq)]
pub enum SaveError {
    /// Ya hay un bloque con ese nombre en esa carpeta.
    Exists(PathBuf),
    /// Nombre no válido, cuerpo vacío o sin carpeta global.
    Invalid(String),
    Io(String),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::Exists(path) => write!(f, "ya existe ({})", path.display()),
            SaveError::Invalid(e) | SaveError::Io(e) => f.write_str(e),
        }
    }
}

/// Los bloques de las dos carpetas. Si uno del proyecto se llama como uno
/// global, manda el del proyecto.
#[derive(Debug, Clone, Default)]
pub struct Library {
    global: Option<PathBuf>,
    project: Option<PathBuf>,
    /// Ordenados por nombre.
    pub blocks: Vec<Block>,
    /// Lo que no se pudo leer (no impide usar los demás).
    pub warnings: Vec<String>,
}

impl Library {
    /// Lee las dos carpetas. La global se crea si falta; la del proyecto,
    /// al guardar el primer bloque en ella.
    pub fn open(global: Option<PathBuf>, project: Option<PathBuf>) -> Library {
        let mut library = Library {
            global,
            project,
            ..Library::default()
        };
        library.reload();
        library
    }

    pub fn dir(&self, scope: Scope) -> Option<&Path> {
        match scope {
            Scope::Global => self.global.as_deref(),
            Scope::Project => self.project.as_deref(),
        }
    }

    /// Vuelve a leer las carpetas.
    pub fn reload(&mut self) {
        self.blocks.clear();
        self.warnings.clear();
        for scope in [Scope::Project, Scope::Global] {
            let Some(dir) = self.dir(scope).map(Path::to_path_buf) else {
                continue;
            };
            // La global se crea si falta; la del proyecto, al guardar en ella
            // (no se siembra una carpeta junto a cada script que se abre).
            if scope == Scope::Global
                && let Err(e) = std::fs::create_dir_all(&dir)
            {
                self.warnings
                    .push(format!("no se pudo crear {}: {e}", dir.display()));
                continue;
            }
            for block in read_dir(&dir, scope, &mut self.warnings) {
                match self.blocks.iter().find(|b| b.name == block.name) {
                    Some(other) if other.scope == scope => self.warnings.push(format!(
                        "{}: el bloque «{}» ya está en {}",
                        block.path.display(),
                        block.name,
                        other.path.display()
                    )),
                    // Uno global tapado por otro del proyecto.
                    Some(_) => {}
                    None => self.blocks.push(block),
                }
            }
        }
        self.blocks.sort_by(|a, b| a.name.cmp(&b.name));
    }

    pub fn get(&self, name: &str) -> Option<&Block> {
        self.blocks.iter().find(|b| b.name == name)
    }

    /// Bloques que coinciden con la búsqueda, por nombre.
    pub fn filter(&self, query: &str) -> Vec<&Block> {
        self.blocks.iter().filter(|b| b.matches(query)).collect()
    }

    /// Guarda un bloque nuevo en la carpeta de `scope` (o, con
    /// `overwrite`, sustituye el que se llame igual allí) y recarga.
    pub fn save(
        &mut self,
        scope: Scope,
        name: &str,
        description: &str,
        tags: &[String],
        body: &str,
        overwrite: bool,
    ) -> Result<Block, SaveError> {
        check_name(name).map_err(SaveError::Invalid)?;
        if body.trim().is_empty() {
            return Err(SaveError::Invalid("el bloque está vacío".into()));
        }
        let Some(dir) = self.dir(scope).map(Path::to_path_buf) else {
            return Err(SaveError::Invalid(
                "no hay carpeta global de bloques (falta $HOME)".into(),
            ));
        };
        let existing = read_dir(&dir, scope, &mut Vec::new())
            .into_iter()
            .find(|b| b.name == name)
            .map(|b| b.path);
        let path = match existing {
            Some(path) if !overwrite => return Err(SaveError::Exists(path)),
            Some(path) => path,
            None => {
                let path = dir.join(format!("{name}.lua"));
                if path.exists() && !overwrite {
                    return Err(SaveError::Exists(path));
                }
                path
            }
        };
        let block = Block {
            name: name.to_string(),
            description: description.trim().to_string(),
            tags: tags.to_vec(),
            body: normalize_body(body),
            scope,
            path,
        };
        std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(&block.path, block.to_file()))
            .map_err(|e| SaveError::Io(format!("{}: {e}", block.path.display())))?;
        self.reload();
        Ok(block)
    }

    /// Cambia el cuerpo de un bloque (conserva su cabecera y su archivo).
    pub fn update(&mut self, name: &str, body: &str) -> Result<Block, SaveError> {
        let Some(block) = self.get(name).cloned() else {
            return Err(SaveError::Invalid(format!("no hay ningún bloque «{name}»")));
        };
        if body.trim().is_empty() {
            return Err(SaveError::Invalid("el bloque está vacío".into()));
        }
        let block = Block {
            body: normalize_body(body),
            ..block
        };
        std::fs::write(&block.path, block.to_file())
            .map_err(|e| SaveError::Io(format!("{}: {e}", block.path.display())))?;
        self.reload();
        Ok(block)
    }

    /// Borra el archivo de un bloque. Si tapaba a uno global con el mismo
    /// nombre, ese vuelve a verse.
    pub fn delete(&mut self, name: &str) -> Result<Block, String> {
        let Some(block) = self.get(name).cloned() else {
            return Err(format!("no hay ningún bloque «{name}»"));
        };
        std::fs::remove_file(&block.path).map_err(|e| format!("{}: {e}", block.path.display()))?;
        self.reload();
        Ok(block)
    }
}

/// Bloques de una carpeta, en orden de archivo (ninguno si no existe).
fn read_dir(dir: &Path, scope: Scope, warnings: &mut Vec<String>) -> Vec<Block> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            warnings.push(format!("no se pudo leer {}: {e}", dir.display()));
            return Vec::new();
        }
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "lua"))
        .collect();
    paths.sort();
    let mut blocks = Vec::new();
    for path in paths {
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) => {
                warnings.push(format!("{}: {e}", path.display()));
                continue;
            }
        };
        let (name, description, tags, body) = parse(&text);
        let name = name.unwrap_or_else(|| {
            path.file_stem()
                .map_or_else(String::new, |s| s.to_string_lossy().into_owned())
        });
        if let Err(e) = check_name(&name) {
            warnings.push(format!("{}: {e}", path.display()));
            continue;
        }
        if body.trim().is_empty() {
            warnings.push(format!("{}: el bloque está vacío", path.display()));
            continue;
        }
        blocks.push(Block {
            name,
            description,
            tags,
            body,
            scope,
            path,
        });
    }
    blocks
}
