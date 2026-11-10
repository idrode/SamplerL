//! Fuzzers en Rust estable para la segunda auditoría.
//!
//!     cargo run --release -- <objetivo> [segundos] [semilla]
//!
//! Objetivos: `wav`, `notacion`, `proyecto`, `bloques`, `lua`, `motor`.
//! Cada uno corre el tiempo dado (60 s por defecto) y cuenta pánicos,
//! iteraciones lentas (más de `LENTO`), picos de memoria (más de `GORDO`) y,
//! en `lua` y `motor`, muestras no finitas o fuera de ±1. Lo que encuentra
//! queda en `hallazgos/<objetivo>-<n>.txt` con la entrada que lo provoca.
//!
//! Se compila con `overflow-checks` y `debug-assertions` aunque sea
//! `--release`: un desborde de enteros es un pánico, no un valor que da la
//! vuelta.

use std::alloc::{GlobalAlloc, Layout, System};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

mod lua;
mod texto;
mod wav;

/// Cuenta lo reservado para ver el pico de cada iteración.
struct Contador;
static AHORA: AtomicUsize = AtomicUsize::new(0);
static PICO: AtomicUsize = AtomicUsize::new(0);
/// Mientras corre `Engine::process` (el arnés es de un solo hilo): cada
/// reserva, liberación o realloc cuenta, y de la primera queda la traza.
static EN_AUDIO: AtomicBool = AtomicBool::new(false);
pub static RESERVAS_AUDIO: AtomicUsize = AtomicUsize::new(0);
pub static TRAZA: Mutex<String> = Mutex::new(String::new());
static TRAZAS: AtomicUsize = AtomicUsize::new(0);

fn en_audio() {
    if EN_AUDIO.load(Ordering::Relaxed) {
        // Resolver una traza tarda segundos: solo las primeras de cada campaña.
        if RESERVAS_AUDIO.fetch_add(1, Ordering::Relaxed) == 0 && TRAZAS.fetch_add(1, Ordering::Relaxed) < 8 {
            // La traza reserva memoria: fuera de la cuenta.
            EN_AUDIO.store(false, Ordering::Relaxed);
            *TRAZA.lock().unwrap() = std::backtrace::Backtrace::force_capture().to_string();
            EN_AUDIO.store(true, Ordering::Relaxed);
        }
    }
}

/// Corre `f` como si fuera el callback de audio: lo que reserve se cuenta.
pub fn audio<R>(f: impl FnOnce() -> R) -> R {
    EN_AUDIO.store(true, Ordering::Relaxed);
    let r = f();
    EN_AUDIO.store(false, Ordering::Relaxed);
    r
}

unsafe impl GlobalAlloc for Contador {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        en_audio();
        let n = AHORA.fetch_add(l.size(), Ordering::Relaxed) + l.size();
        PICO.fetch_max(n, Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        en_audio();
        AHORA.fetch_sub(l.size(), Ordering::Relaxed);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, size: usize) -> *mut u8 {
        en_audio();
        if size > l.size() {
            let n = AHORA.fetch_add(size - l.size(), Ordering::Relaxed) + size - l.size();
            PICO.fetch_max(n, Ordering::Relaxed);
        } else {
            AHORA.fetch_sub(l.size() - size, Ordering::Relaxed);
        }
        unsafe { System.realloc(p, l, size) }
    }
}

#[global_allocator]
static GLOBAL: Contador = Contador;

pub const LENTO: Duration = Duration::from_secs(3);
pub const GORDO: usize = 512 << 20;

/// xorshift64*: determinista y sin dependencias.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
    pub fn chance(&mut self, p: f64) -> bool {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 <= p
    }
}

/// Lo que cuenta una campaña.
#[derive(Default)]
pub struct Cuenta {
    pub iteraciones: u64,
    pub aceptadas: u64,
    pub panicos: u64,
    pub lentas: u64,
    pub gordas: u64,
    pub no_finitas: u64,
    pub fuera: u64,
    pub con_reservas: u64,
    pub guardados: u64,
    pub peor_tiempo: Duration,
    pub peor_pico: usize,
}

static ULTIMO_PANICO: Mutex<String> = Mutex::new(String::new());

pub fn carpeta(nombre: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(nombre);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

impl Cuenta {
    /// Corre una iteración: `f` devuelve si la entrada se aceptó. Guarda la
    /// entrada si hay pánico, si tarda o si reserva demasiado.
    pub fn correr(&mut self, objetivo: &str, entrada: &[u8], f: impl FnOnce() -> bool) {
        self.iteraciones += 1;
        // Por si la iteración no vuelve (bucle sin fin, desborde de pila):
        // la entrada queda escrita antes de empezar.
        let _ = std::fs::write(carpeta("tmp").join(format!("{objetivo}-actual.txt")), entrada);
        EN_AUDIO.store(false, Ordering::Relaxed);
        RESERVAS_AUDIO.store(0, Ordering::Relaxed);
        TRAZA.lock().unwrap().clear();
        PICO.store(AHORA.load(Ordering::Relaxed), Ordering::Relaxed);
        let base = AHORA.load(Ordering::Relaxed);
        let t = Instant::now();
        let r = catch_unwind(AssertUnwindSafe(f));
        let dt = t.elapsed();
        EN_AUDIO.store(false, Ordering::Relaxed);
        let pico = PICO.load(Ordering::Relaxed).saturating_sub(base);
        self.peor_tiempo = self.peor_tiempo.max(dt);
        self.peor_pico = self.peor_pico.max(pico);
        let mut motivo = None;
        match r {
            Ok(true) => self.aceptadas += 1,
            Ok(false) => {}
            Err(_) => {
                self.panicos += 1;
                motivo = Some(format!("PÁNICO: {}", ULTIMO_PANICO.lock().unwrap()));
            }
        }
        if dt > LENTO {
            self.lentas += 1;
            motivo.get_or_insert(format!("LENTA: {dt:?}"));
        }
        if pico > GORDO {
            self.gordas += 1;
            motivo.get_or_insert(format!("MEMORIA: {} MB", pico >> 20));
        }
        let reservas = RESERVAS_AUDIO.load(Ordering::Relaxed);
        if reservas > 0 {
            self.con_reservas += 1;
            let traza = TRAZA.lock().unwrap().clone();
            let propias: Vec<&str> = traza.lines().filter(|l| l.contains("engine::") || l.contains("/engine/src/")).take(14).collect();
            motivo.get_or_insert(format!(
                "RESERVA EN EL HILO DE AUDIO: {reservas} llamadas al asignador dentro de Engine::process; la primera:\n{}",
                propias.join("\n")
            ));
        }
        if let Some(motivo) = motivo {
            self.guardar(objetivo, &motivo, entrada);
        }
    }

    pub fn guardar(&mut self, objetivo: &str, motivo: &str, entrada: &[u8]) {
        self.guardados += 1;
        if self.guardados > 40 {
            return;
        }
        let ruta = carpeta("hallazgos").join(format!("{objetivo}-{:03}.txt", self.guardados));
        let mut bytes = format!("### {motivo}\n").into_bytes();
        bytes.extend_from_slice(entrada);
        let _ = std::fs::write(&ruta, bytes);
        eprintln!("  {} → {}", motivo.lines().next().unwrap_or(""), ruta.display());
    }

    pub fn resumen(&self, objetivo: &str, segundos: u64) {
        println!(
            "{objetivo}: {} iteraciones en {segundos} s, {} aceptadas, {} pánicos, {} lentas (>{:?}), \
             {} con más de {} MB, {} con muestras no finitas o motor envenenado, {} con muestras fuera de ±1, \
             {} con reservas en el hilo de audio; \
             peor tiempo {:?}, peor pico {} MB",
            self.iteraciones,
            self.aceptadas,
            self.panicos,
            self.lentas,
            LENTO,
            self.gordas,
            GORDO >> 20,
            self.no_finitas,
            self.fuera,
            self.con_reservas,
            self.peor_tiempo,
            self.peor_pico >> 20,
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let objetivo = args.get(1).map(String::as_str).unwrap_or("");
    let segundos: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(60);
    let semilla: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0x5EED_1234_ABCD_0001);
    std::panic::set_hook(Box::new(|info| {
        *ULTIMO_PANICO.lock().unwrap() = info.to_string();
    }));
    let mut rng = Rng(semilla | 1);
    let mut cuenta = Cuenta::default();
    let fin = Instant::now() + Duration::from_secs(segundos);
    match objetivo {
        "wav" => wav::correr(&mut rng, &mut cuenta, fin),
        "notacion" => texto::notacion(&mut rng, &mut cuenta, fin),
        "proyecto" => texto::proyecto(&mut rng, &mut cuenta, fin),
        "bloques" => texto::bloques(&mut rng, &mut cuenta, fin),
        "lua" => lua::correr(&mut rng, &mut cuenta, fin, false),
        "motor" => lua::correr(&mut rng, &mut cuenta, fin, true),
        _ => {
            eprintln!("uso: auditoria-fuzz <wav|notacion|proyecto|bloques|lua|motor> [segundos] [semilla]");
            std::process::exit(2);
        }
    }
    cuenta.resumen(objetivo, segundos);
}
