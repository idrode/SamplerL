//! Un valor de un parámetro con su unidad, para enseñarlo: Hz (kHz desde
//! mil) en las frecuencias, dB, ms y semitonos. Solo es texto: lo que suena
//! no se redondea.

/// La unidad de un parámetro por su nombre (el de después del punto en
/// `lowpass.cutoff`).
pub fn unit(param: &str) -> &'static str {
    let name = param.rsplit('.').next().unwrap_or(param);
    let name = name.trim_end_matches(|c: char| c.is_ascii_digit());
    match name {
        "cutoff" | "freq" | "tone" => "Hz",
        "threshold" | "makeup" | "range" | "gain" if param.contains("eq") || name != "gain" => "dB",
        "attack" | "release" | "decay" | "bend_time" | "crossfade" | "glide" => "ms",
        "pitch" | "bend" | "formant" => "st",
        _ => "",
    }
}

/// `1.23 kHz`, `-12.0 dB`, `0.73`.
pub fn with_unit(param: &str, value: f32) -> String {
    let number = crate::lanes_panel::number;
    match unit(param) {
        "Hz" if value.abs() >= 1000.0 => format!("{:.2} kHz", value / 1000.0),
        "" => number(value),
        u => format!("{} {u}", number(value)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_kind_with_its_unit() {
        assert_eq!(with_unit("lowpass.cutoff", 1234.5), "1.23 kHz");
        assert_eq!(with_unit("cutoff", 640.0), "640 Hz");
        assert_eq!(with_unit("eq.gain2", -6.0), "-6.00 dB");
        assert_eq!(with_unit("compressor.threshold", -20.0), "-20.0 dB");
        assert_eq!(with_unit("gain", 0.73), "0.73");
        assert_eq!(with_unit("slice.bend", -12.0), "-12.0 st");
        assert_eq!(with_unit("slice.release", 250.0), "250 ms");
        assert_eq!(with_unit("send.reverb", 0.3), "0.30");
    }
}
