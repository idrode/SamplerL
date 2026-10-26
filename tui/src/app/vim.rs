//! Modo vim del editor (fase R, fase 1), con `[editor] modo = "vim"`,
//! sobre `ratatui-textarea` (que no tiene modos): una capa que en Normal
//! convierte las teclas en movimientos y órdenes del widget y en Insert las
//! deja escribir como siempre.
//!
//! Normal: `h j k l w b e 0 $ gg G`; `i a o O A I`; `x dd yy p P`; `u` y
//! Ctrl-R; `/` con `n` y `N`; `:w`. Sin cuentas, macros, registros, `:s`,
//! marcas, operadores compuestos, Visual ni `.` (para después).
//!
//! Esc sigue pasando del editor al modo tocar también aquí (decidido por
//! el usuario): en Insert, Esc no vuelve a Normal; lo hace `jk` (dos
//! teclas seguidas, configurable con `[editor] salir_insert`). Ctrl-[ no
//! sirve: la terminal lo manda como el mismo byte que Esc.

use ratatui_textarea::{CursorMove, Input, Key};

use super::{App, Flow};
use crate::palette::Action;

/// El modo del editor vim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VimMode {
    Normal,
    Insert,
}

/// Lo que lleva la capa vim entre tecla y tecla.
#[derive(Debug, Clone, PartialEq)]
pub struct Vim {
    pub mode: VimMode,
    /// La primera tecla de una orden de dos (`g`, `d`, `y`).
    pending: Option<char>,
    /// Lo copiado por `yy` o `dd` (entero, por líneas): `p` y `P` lo ponen
    /// en una línea propia.
    linewise: bool,
    /// Las dos teclas que salen de Insert y si la primera acaba de
    /// escribirse.
    exit: [char; 2],
    half_exit: bool,
}

impl Vim {
    pub fn new(exit: [char; 2]) -> Vim {
        Vim {
            mode: VimMode::Normal,
            pending: None,
            linewise: false,
            exit,
            half_exit: false,
        }
    }

    pub fn set_exit(&mut self, exit: [char; 2]) {
        self.exit = exit;
        self.half_exit = false;
    }

    /// Cómo se ve el modo en la barra de estado.
    pub fn label(&self) -> &'static str {
        match self.mode {
            VimMode::Normal => "NORMAL",
            VimMode::Insert => "INSERT",
        }
    }
}

/// Las dos teclas para salir de Insert por defecto.
pub const DEFAULT_EXIT: [char; 2] = ['j', 'k'];

impl App {
    /// Enciende o apaga el modo vim (para esta sesión; el de siempre es el
    /// de `[editor] modo`).
    pub(super) fn toggle_vim(&mut self) {
        self.vim = match self.vim.take() {
            Some(_) => {
                self.message = "editor clásico".into();
                None
            }
            None => {
                self.message = "modo vim: NORMAL (i escribe, jk vuelve)".into();
                Some(Vim::new(self.vim_exit))
            }
        };
    }

    /// Una tecla en el editor con el modo vim. `None`: que siga por el
    /// camino de siempre (escribir en Insert, los atajos, Esc al modo
    /// tocar...).
    pub(super) fn handle_vim(&mut self, input: &Input, bound: bool) -> Option<Flow> {
        let vim = self.vim.as_mut()?;
        match vim.mode {
            VimMode::Insert => {
                let typed = match input.key {
                    Key::Char(c) if !input.ctrl && !input.alt => Some(c),
                    _ => None,
                };
                if vim.half_exit && typed == Some(vim.exit[1]) {
                    // La primera ya se escribió: se borra y a Normal.
                    vim.half_exit = false;
                    vim.mode = VimMode::Normal;
                    self.completion = None;
                    self.editor.delete_char();
                    // Como en vim, al salir el cursor va a la izquierda.
                    self.vim_left();
                    return Some(Flow::Continue);
                }
                vim.half_exit = typed == Some(vim.exit[0]);
                None
            }
            VimMode::Normal => {
                // Los atajos (Ctrl-S, Esc al modo tocar, la paleta...) y las
                // teclas que no son letras (flechas, Inicio...) van como
                // siempre.
                let Key::Char(c) = input.key else {
                    vim.pending = None;
                    return match input.key {
                        Key::Enter if !bound => {
                            self.editor.move_cursor(CursorMove::Down);
                            self.first_non_blank();
                            Some(Flow::Continue)
                        }
                        Key::Backspace if !bound => {
                            self.vim_left();
                            Some(Flow::Continue)
                        }
                        Key::Tab | Key::Delete if !bound => Some(Flow::Continue),
                        _ => None,
                    };
                };
                if input.ctrl || input.alt || bound {
                    vim.pending = None;
                    return None;
                }
                Some(self.vim_normal(c))
            }
        }
    }

    /// Una letra en Normal.
    fn vim_normal(&mut self, c: char) -> Flow {
        let Some(vim) = self.vim.as_mut() else {
            return Flow::Continue;
        };
        let pending = vim.pending.take();
        match (pending, c) {
            (Some('g'), 'g') => self.editor.move_cursor(CursorMove::Top),
            (Some('d'), 'd') => self.delete_line(),
            (Some('y'), 'y') => self.yank_line(),
            (Some(first), _) => {
                self.message = format!("vim: «{first}{c}» no está (fase 1)");
            }
            (None, 'g' | 'd' | 'y') => vim.pending = Some(c),
            (None, 'h') => self.vim_left(),
            (None, 'l') => self.vim_right(),
            (None, 'j') => self.editor.move_cursor(CursorMove::Down),
            (None, 'k') => self.editor.move_cursor(CursorMove::Up),
            (None, 'w') => self.editor.move_cursor(CursorMove::WordForward),
            (None, 'b') => self.editor.move_cursor(CursorMove::WordBack),
            (None, 'e') => self.editor.move_cursor(CursorMove::WordEnd),
            (None, '0') => self.editor.move_cursor(CursorMove::Head),
            (None, '$') => {
                self.editor.move_cursor(CursorMove::End);
                self.normal_cursor();
            }
            (None, 'G') => {
                self.editor.move_cursor(CursorMove::Bottom);
                self.first_non_blank();
            }
            (None, 'i') => self.insert(),
            (None, 'a') => {
                if self.editor.cursor().1 < self.line_len() {
                    self.editor.move_cursor(CursorMove::Forward);
                }
                self.insert();
            }
            (None, 'A') => {
                self.editor.move_cursor(CursorMove::End);
                self.insert();
            }
            (None, 'I') => {
                self.first_non_blank();
                self.insert();
            }
            (None, 'o') => {
                self.grouped(|e| {
                    e.move_cursor(CursorMove::End);
                    e.insert_newline();
                });
                self.insert();
            }
            (None, 'O') => {
                self.grouped(|e| {
                    e.move_cursor(CursorMove::Head);
                    e.insert_newline();
                    e.move_cursor(CursorMove::Up);
                });
                self.insert();
            }
            (None, 'x') => self.delete_under(),
            (None, 'p') => self.put(true),
            (None, 'P') => self.put(false),
            (None, 'u') => {
                self.run_action(Action::Undo);
                self.normal_cursor();
            }
            (None, '/') => {
                self.run_action(Action::Find);
            }
            (None, 'n') => {
                self.run_action(Action::FindNext);
            }
            (None, 'N') => {
                self.run_action(Action::FindPrevious);
            }
            (None, ':') => {
                self.prompt = Some(":".into());
                self.message = "vim: :w guarda (y carga, como Ctrl-S)".into();
            }
            (None, _) => {}
        }
        Flow::Continue
    }

    /// Una orden de `:` (la línea de órdenes empieza por `:`).
    pub(super) fn vim_command(&mut self, line: &str) {
        match line.trim() {
            "w" => self.save(),
            other => self.message = format!("vim: «:{other}» no está (fase 1: solo :w)"),
        }
    }

    fn insert(&mut self) {
        if let Some(vim) = &mut self.vim {
            vim.mode = VimMode::Insert;
            vim.half_exit = false;
        }
    }

    /// Lo que mide la línea del cursor, en caracteres.
    fn line_len(&self) -> usize {
        let row = self.editor.cursor().0;
        self.editor.lines()[row].chars().count()
    }

    /// En Normal el cursor está sobre un carácter: no detrás del último.
    fn normal_cursor(&mut self) {
        let col = self.editor.cursor().1;
        if col > 0 && col >= self.line_len() {
            self.editor.move_cursor(CursorMove::Back);
        }
    }

    /// `h`: a la izquierda sin pasar a la línea anterior.
    fn vim_left(&mut self) {
        if self.editor.cursor().1 > 0 {
            self.editor.move_cursor(CursorMove::Back);
        }
    }

    /// `l`: a la derecha sin pasar del último carácter.
    fn vim_right(&mut self) {
        if self.editor.cursor().1 + 1 < self.line_len() {
            self.editor.move_cursor(CursorMove::Forward);
        }
    }

    /// Al primer carácter que no es un espacio de la línea.
    fn first_non_blank(&mut self) {
        let row = self.editor.cursor().0;
        let col = self.editor.lines()[row]
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();
        self.jump_to(row, col);
    }

    fn jump_to(&mut self, row: usize, col: usize) {
        let clamp = |n: usize| n.min(u16::MAX as usize) as u16;
        self.editor
            .move_cursor(CursorMove::Jump(clamp(row), clamp(col)));
    }

    /// `x`: borra el carácter del cursor (y lo copia).
    fn delete_under(&mut self) {
        let at = self.editor.cursor();
        let (row, col) = (at.0, at.1);
        let Some(c) = self.editor.lines()[row].chars().nth(col) else {
            return;
        };
        self.grouped(|e| e.delete_next_char());
        self.editor.set_yank_text(c.to_string());
        if let Some(vim) = &mut self.vim {
            vim.linewise = false;
        }
        self.normal_cursor();
    }

    /// `yy`: copia la línea entera.
    fn yank_line(&mut self) {
        let row = self.editor.cursor().0;
        let line = self.editor.lines()[row].clone();
        self.editor.set_yank_text(line);
        if let Some(vim) = &mut self.vim {
            vim.linewise = true;
        }
        self.message = "vim: línea copiada".into();
    }

    /// `dd`: borra la línea entera (y la copia).
    fn delete_line(&mut self) {
        let row = self.editor.cursor().0;
        let lines = self.editor.lines().len();
        let line = self.editor.lines()[row].clone();
        let previous = row
            .checked_sub(1)
            .map(|r| self.editor.lines()[r].chars().count());
        self.grouped(|e| {
            let clamp = |n: usize| n.min(u16::MAX as usize) as u16;
            match (lines, previous) {
                // La única: se vacía.
                (1, _) => {
                    e.move_cursor(CursorMove::Head);
                    e.start_selection();
                    e.move_cursor(CursorMove::End);
                }
                // La última: desde el final de la anterior.
                (n, Some(len)) if row + 1 == n => {
                    e.move_cursor(CursorMove::Jump(clamp(row - 1), clamp(len)));
                    e.start_selection();
                    e.move_cursor(CursorMove::Jump(clamp(row), u16::MAX));
                }
                _ => {
                    e.move_cursor(CursorMove::Jump(clamp(row), 0));
                    e.start_selection();
                    e.move_cursor(CursorMove::Jump(clamp(row + 1), 0));
                }
            }
            e.cut();
        });
        self.editor.set_yank_text(line);
        if let Some(vim) = &mut self.vim {
            vim.linewise = true;
        }
        let row = row.min(self.editor.lines().len() - 1);
        self.jump_to(row, 0);
        self.first_non_blank();
    }

    /// `p` (detrás) y `P` (delante): lo copiado; por líneas, en una línea
    /// propia debajo o encima.
    fn put(&mut self, after: bool) {
        let text = self.editor.yank_text();
        if text.is_empty() {
            return;
        }
        let linewise = self.vim.as_ref().is_some_and(|v| v.linewise);
        let at = self.editor.cursor();
        let (row, col) = (at.0, at.1);
        if linewise {
            self.grouped(|e| {
                if after {
                    e.move_cursor(CursorMove::End);
                    e.insert_newline();
                    e.insert_str(&text);
                } else {
                    e.move_cursor(CursorMove::Head);
                    e.insert_str(&text);
                    e.insert_newline();
                }
            });
            let row = if after { row + 1 } else { row };
            self.jump_to(row, 0);
            self.first_non_blank();
        } else {
            let at = if after && col < self.line_len() {
                col + 1
            } else {
                col
            };
            self.jump_to(row, at);
            self.grouped(|e| e.insert_str(&text));
            self.editor.move_cursor(CursorMove::Back);
        }
    }
}
