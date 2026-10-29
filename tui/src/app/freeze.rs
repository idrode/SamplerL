//! Congelar y descongelar la pista seleccionada (paleta): «Congelar»
//! renderiza su salida tras sus efectos (`Song::render_frozen`) a
//! `congelado/<pista>.wav` en la carpeta de samples y añade al final del
//! script `frozen("pista", "congelado/pista.wav", "huella")`; «Descongelar»
//! quita esa línea. El resto del texto no se toca, cada cosa es un paso de
//! deshacer (Ctrl-Z y Ctrl-S también descongelan) y se carga enseguida
//! (entra al compás, como cualquier cambio).

use super::App;

/// Carpeta de los WAV congelados, dentro de la de samples.
pub const FROZEN_DIR: &str = "congelado";

impl App {
    /// «Congelar la pista seleccionada».
    pub fn freeze_selected(&mut self) {
        let Some(name) = self.tracks().get(self.selected).map(|t| t.name.clone()) else {
            self.message = "no hay pistas".into();
            return;
        };
        if self.error.is_some() || self.loaded_text.as_deref() != Some(self.text().as_str()) {
            self.message = "congelar: el texto no es el cargado; Ctrl-S para cargarlo antes".into();
            return;
        }
        let Some(song) = self.host.song().cloned() else {
            self.message = "congelar: no hay nada cargado".into();
            return;
        };
        let render = match song.render_frozen(self.sample_rate, &name) {
            Ok(render) => render,
            Err(e) => {
                self.message = e.to_string();
                return;
            }
        };
        let file: String = name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let wav = format!("{FROZEN_DIR}/{file}.wav");
        let path = self.sample_folder().join(&wav);
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .map_err(|e| e.to_string())
            .and_then(|()| {
                engine::offline::write_wav_float(&path, &render.audio, 2, self.sample_rate)
                    .map_err(|e| e.to_string())
            });
        if let Err(e) = written {
            self.message = format!("congelar: no se pudo escribir {}: {e}", path.display());
            return;
        }
        let line = format!(
            "frozen(\"{name}\", \"{wav}\", \"{}\") -- congelada: F1 «Descongelar»",
            render.fingerprint
        );
        let mut text = self.text();
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&line);
        text.push('\n');
        self.completion = None;
        self.marks.clear();
        self.replace_text(&text);
        self.load();
        let bars = if render.bars == 1 {
            "1 compás".to_string()
        } else {
            format!("{} compases", render.bars)
        };
        let warnings = if render.warnings.is_empty() {
            String::new()
        } else {
            format!(" · ⚠ {}", render.warnings.join(" · "))
        };
        self.message = format!(
            "{name} congelada en {wav} ({bars}): «Descongelar» la devuelve (o Ctrl-Z y Ctrl-S){warnings}"
        );
    }

    /// «Descongelar la pista seleccionada»: quita su `frozen(...)` del
    /// script (el WAV se queda en la carpeta).
    pub fn unfreeze_selected(&mut self) {
        let Some(name) = self.tracks().get(self.selected).map(|t| t.name.clone()) else {
            self.message = "no hay pistas".into();
            return;
        };
        let call = format!("frozen(\"{name}\"");
        let lines: Vec<String> = self.editor.lines().to_vec();
        let kept: Vec<&String> = lines
            .iter()
            .filter(|l| !l.trim_start().starts_with(&call))
            .collect();
        if kept.len() == lines.len() {
            self.message = format!("{name} no está congelada (no hay {call}...) en el script)");
            return;
        }
        let mut text = kept
            .iter()
            .map(|l| l.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        text.push('\n');
        self.completion = None;
        self.marks.clear();
        self.replace_text(&text);
        self.load();
        self.message = format!(
            "{name} descongelada: vuelve a sonar desde el script (Ctrl-Z y Ctrl-S la congelan otra vez)"
        );
    }
}
