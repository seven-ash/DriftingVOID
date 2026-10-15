//! OKHSV <-> sRGB (Björn Ottosson's construction on top of OKLab), so hue and
//! value steps look perceptually even in the palette.

use egui::Color32;

fn srgb_to_linear(x: f32) -> f32 {
    if x >= 0.04045 { ((x + 0.055) / 1.055).powf(2.4) } else { x / 12.92 }
}

fn linear_to_srgb(x: f32) -> f32 {
    if x >= 0.0031308 { 1.055 * x.powf(1.0 / 2.4) - 0.055 } else { 12.92 * x }
}

fn lin_to_oklab(r: f32, g: f32, b: f32) -> [f32; 3] {
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    [
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    ]
}

fn oklab_to_lin(l: f32, a: f32, b: f32) -> [f32; 3] {
    let l_ = l + 0.3963377774 * a + 0.2158037573 * b;
    let m_ = l - 0.1055613458 * a - 0.0638541728 * b;
    let s_ = l - 0.0894841775 * a - 1.2914855480 * b;
    let (l, m, s) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    [
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
    ]
}

fn max_saturation(a: f32, b: f32) -> f32 {
    let (k0, k1, k2, k3, k4, wl, wm, ws) = if -1.88170328 * a - 0.80936493 * b > 1.0 {
        (1.19086277, 1.76576728, 0.59662641, 0.75515197, 0.56771245, 4.0767416621, -3.3077115913, 0.2309699292)
    } else if 1.81444104 * a - 1.19445276 * b > 1.0 {
        (0.73956515, -0.45954404, 0.08285427, 0.12541070, 0.14503204, -1.2684380046, 2.6097574011, -0.3413193965)
    } else {
        (1.35733652, -0.00915799, -1.15130210, -0.50559606, 0.00692167, -0.0041960863, -0.7034186147, 1.7076147010)
    };
    let mut s = k0 + k1 * a + k2 * b + k3 * a * a + k4 * a * b;
    let kl = 0.3963377774 * a + 0.2158037573 * b;
    let km = -0.1055613458 * a - 0.0638541728 * b;
    let ks = -0.0894841775 * a - 1.2914855480 * b;
    let l_ = 1.0 + s * kl;
    let m_ = 1.0 + s * km;
    let s_ = 1.0 + s * ks;
    let (l, m, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    let (ld, md, sd) = (3.0 * kl * l_ * l_, 3.0 * km * m_ * m_, 3.0 * ks * s_ * s_);
    let (ld2, md2, sd2) = (6.0 * kl * kl * l_, 6.0 * km * km * m_, 6.0 * ks * ks * s_);
    let f = wl * l + wm * m + ws * s3;
    let f1 = wl * ld + wm * md + ws * sd;
    let f2 = wl * ld2 + wm * md2 + ws * sd2;
    s -= f * f1 / (f1 * f1 - 0.5 * f * f2);
    s
}

/// (L, C) of the most saturated in-gamut colour for a hue direction.
fn cusp(a: f32, b: f32) -> (f32, f32) {
    let s = max_saturation(a, b);
    let rgb = oklab_to_lin(1.0, s * a, s * b);
    let l = (1.0 / rgb[0].max(rgb[1]).max(rgb[2])).cbrt();
    (l, l * s)
}

const K1: f32 = 0.206;
const K2: f32 = 0.03;
const K3: f32 = (1.0 + K1) / (1.0 + K2);

fn toe(x: f32) -> f32 {
    0.5 * (K3 * x - K1 + ((K3 * x - K1).powi(2) + 4.0 * K2 * K3 * x).sqrt())
}

fn toe_inv(x: f32) -> f32 {
    (x * x + K1 * x) / (K3 * (x + K2))
}

/// h, s, v in 0..1.
pub fn to_rgb(h: f32, s: f32, v: f32) -> Color32 {
    let s = s.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);
    if v <= 1e-5 {
        return Color32::BLACK;
    }
    let a_ = (std::f32::consts::TAU * h).cos();
    let b_ = (std::f32::consts::TAU * h).sin();
    let (lc, cc) = cusp(a_, b_);
    let (s_max, t_max) = (cc / lc, cc / (1.0 - lc));
    let s0 = 0.5;
    let k = 1.0 - s0 / s_max;
    let lv = 1.0 - s * s0 / (s0 + t_max - t_max * k * s);
    let cv = s * t_max * s0 / (s0 + t_max - t_max * k * s);
    let mut l = v * lv;
    let mut c = v * cv;
    let lvt = toe_inv(lv);
    let cvt = cv * lvt / lv;
    let l_new = toe_inv(l);
    c = c * l_new / l;
    l = l_new;
    let rs = oklab_to_lin(lvt, a_ * cvt, b_ * cvt);
    let scale = (1.0 / rs[0].max(rs[1]).max(rs[2]).max(0.0)).cbrt();
    l *= scale;
    c *= scale;
    let rgb = oklab_to_lin(l, c * a_, c * b_);
    let u = |x: f32| (linear_to_srgb(x.clamp(0.0, 1.0)) * 255.0 + 0.5) as u8;
    Color32::from_rgb(u(rgb[0]), u(rgb[1]), u(rgb[2]))
}

/// Returns (h, s, v); `hue_hint` is kept for greys, where hue is undefined.
pub fn from_rgb(c: Color32, hue_hint: f32) -> (f32, f32, f32) {
    let [r, g, b, _] = c.to_srgba_unmultiplied();
    let f = |x: u8| srgb_to_linear(x as f32 / 255.0);
    let lab = lin_to_oklab(f(r), f(g), f(b));
    let chroma = (lab[1] * lab[1] + lab[2] * lab[2]).sqrt();
    if chroma < 1e-4 {
        return (hue_hint, 0.0, toe(lab[0]).clamp(0.0, 1.0));
    }
    let (a_, b_) = (lab[1] / chroma, lab[2] / chroma);
    let mut l = lab[0];
    let c = chroma;
    let h = 0.5 + 0.5 * (-lab[2]).atan2(-lab[1]) / std::f32::consts::PI;
    let (lc, cc) = cusp(a_, b_);
    let (s_max, t_max) = (cc / lc, cc / (1.0 - lc));
    let s0 = 0.5;
    let k = 1.0 - s0 / s_max;
    let t = t_max / (c + l * t_max);
    let lv = t * l;
    let cv = t * c;
    let lvt = toe_inv(lv);
    let cvt = cv * lvt / lv;
    let rs = oklab_to_lin(lvt, a_ * cvt, b_ * cvt);
    let scale = (1.0 / rs[0].max(rs[1]).max(rs[2]).max(0.0)).cbrt();
    l /= scale;
    l = toe(l);
    let v = l / lv;
    let s = (s0 + t_max) * cv / (t_max * s0 + t_max * k * cv);
    (h, s.clamp(0.0, 1.0), v.clamp(0.0, 1.0))
}

/// OKLab of an OKHSV colour (via its gamut-mapped sRGB value).
pub fn to_oklab(h: f32, s: f32, v: f32) -> [f32; 3] {
    let [r, g, b, _] = to_rgb(h, s, v).to_array();
    let f = |x: u8| srgb_to_linear(x as f32 / 255.0);
    lin_to_oklab(f(r), f(g), f(b))
}

/// OKLCH (hue 0..1) to sRGB; the bool says whether it was inside the gamut.
/// Out-of-gamut colours come back clamped.
pub fn oklch(l: f32, c: f32, h: f32) -> (Color32, bool) {
    let a = c * (std::f32::consts::TAU * h).cos();
    let b = c * (std::f32::consts::TAU * h).sin();
    let rgb = oklab_to_lin(l, a, b);
    let inside = rgb.iter().all(|v| (-0.0005..=1.0005).contains(v));
    let u = |x: f32| (linear_to_srgb(x.clamp(0.0, 1.0)) * 255.0 + 0.5) as u8;
    (Color32::from_rgb(u(rgb[0]), u(rgb[1]), u(rgb[2])), inside)
}

/// OKLab lightness, chroma and hue (0..1); greys keep `hue_hint`.
pub fn lch_of(c: Color32, hue_hint: f32) -> (f32, f32, f32) {
    let [r, g, b, _] = c.to_srgba_unmultiplied();
    let f = |x: u8| srgb_to_linear(x as f32 / 255.0);
    let lab = lin_to_oklab(f(r), f(g), f(b));
    let chroma = (lab[1] * lab[1] + lab[2] * lab[2]).sqrt();
    let h = if chroma < 1e-4 { hue_hint } else { (lab[2].atan2(lab[1]) / std::f32::consts::TAU).rem_euclid(1.0) };
    (lab[0], chroma, h)
}

#[cfg(test)]
mod tests {
    #[test]
    fn roundtrip() {
        for c in [egui::Color32::from_rgb(230, 57, 70), egui::Color32::from_rgb(58, 134, 255), egui::Color32::from_rgb(255, 209, 59)] {
            let (h, s, v) = super::from_rgb(c, 0.0);
            let d = super::to_rgb(h, s, v);
            for (a, b) in c.to_array().iter().zip(d.to_array().iter()) {
                assert!((*a as i32 - *b as i32).abs() <= 2, "{c:?} -> {d:?}");
            }
        }
    }
}
