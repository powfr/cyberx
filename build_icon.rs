// Dessine le logo CyberX (C et Y entrelacés) et produit un fichier .ico,
// sans bibliothèque ni image binaire dans le dépôt. Même géométrie que assets/logo.svg.

const C_CENTER: (f64, f64) = (222.0, 262.0);
const C_RADIUS: f64 = 158.0;
const C_HALF: f64 = 31.0; // demi-épaisseur du C
const C_GAP: (f64, f64) = (-36.0, 24.0); // ouverture du C (degrés), côté droit
const Y_HALF: f64 = 28.0; // demi-épaisseur du Y
const Y_SEGS: [((f64, f64), (f64, f64)); 3] = [
    ((240.0, 62.0), (344.0, 262.0)),
    ((448.0, 62.0), (344.0, 262.0)),
    ((344.0, 262.0), (344.0, 452.0)),
];

fn seg_dist(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}

fn y_dist(p: (f64, f64)) -> f64 {
    Y_SEGS.iter().map(|(a, b)| seg_dist(p, *a, *b)).fold(f64::MAX, f64::min)
}

fn c_dist(p: (f64, f64)) -> f64 {
    let (dx, dy) = (p.0 - C_CENTER.0, p.1 - C_CENTER.1);
    let ang = dy.atan2(dx).to_degrees();
    if ang > C_GAP.0 && ang < C_GAP.1 {
        // Dans l'ouverture : distance aux deux extrémités arrondies.
        let end = |deg: f64| {
            let r = deg.to_radians();
            (C_CENTER.0 + C_RADIUS * r.cos(), C_CENTER.1 + C_RADIUS * r.sin())
        };
        let d = |e: (f64, f64)| ((p.0 - e.0).powi(2) + (p.1 - e.1).powi(2)).sqrt();
        d(end(C_GAP.0)).min(d(end(C_GAP.1)))
    } else {
        ((dx * dx + dy * dy).sqrt() - C_RADIUS).abs()
    }
}

fn lerp(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Image RGBA (non prémultipliée) de `size`×`size` pixels.
pub fn render(size: u32) -> Vec<u8> {
    let scale = 512.0 / size as f64;
    let aa = scale; // largeur d'anticrénelage, en unités du dessin
    let cov = |half: f64, d: f64| ((half - d) / aa + 0.5).clamp(0.0, 1.0);
    let c_col = ([70.0, 244.0, 255.0], [0.0, 120.0, 255.0]);
    let y_col = ([255.0, 82.0, 220.0], [164.0, 20.0, 255.0]);
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for j in 0..size {
        for i in 0..size {
            let p = ((i as f64 + 0.5) * scale, (j as f64 + 0.5) * scale);
            let (dc, dy) = (c_dist(p), y_dist(p));
            // Haut : le Y passe devant le C. Bas : le C passe devant le Y.
            let mut a_c = cov(C_HALF, dc);
            if p.1 < 215.0 {
                a_c *= 1.0 - cov(44.0, dy);
            }
            let mut a_y = cov(Y_HALF, dy);
            if p.0 >= 290.0 && p.1 >= 236.0 {
                a_y *= 1.0 - cov(46.0, dc);
            }
            let cc = lerp(c_col.0, c_col.1, (p.0 + p.1 - 160.0) / 560.0);
            let yc = lerp(y_col.0, y_col.1, (p.0 * 0.4 + p.1 - 120.0) / 520.0);
            // Composition : Y par-dessus C.
            let a = a_y + a_c * (1.0 - a_y);
            let mut rgb = [0.0; 3];
            if a > 0.0 {
                for k in 0..3 {
                    rgb[k] = (yc[k] * a_y + cc[k] * a_c * (1.0 - a_y)) / a;
                }
            }
            out.extend_from_slice(&[rgb[0] as u8, rgb[1] as u8, rgb[2] as u8, (a * 255.0).round() as u8]);
        }
    }
    out
}

/// Fichier .ico avec plusieurs tailles (images BMP 32 bits).
pub fn ico(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes
        .iter()
        .map(|&s| {
            let rgba = render(s);
            let mask_row = ((s + 31) / 32 * 4) as usize;
            let mut bmp = Vec::new();
            let le32 = |v: u32| v.to_le_bytes();
            bmp.extend_from_slice(&le32(40));
            bmp.extend_from_slice(&le32(s));
            bmp.extend_from_slice(&le32(s * 2));
            bmp.extend_from_slice(&1u16.to_le_bytes());
            bmp.extend_from_slice(&32u16.to_le_bytes());
            bmp.extend_from_slice(&[0u8; 24]);
            for j in (0..s).rev() {
                for i in 0..s {
                    let o = ((j * s + i) * 4) as usize;
                    bmp.extend_from_slice(&[rgba[o + 2], rgba[o + 1], rgba[o], rgba[o + 3]]);
                }
            }
            bmp.extend(std::iter::repeat(0u8).take(mask_row * s as usize));
            bmp
        })
        .collect();
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (s, img) in sizes.iter().zip(&images) {
        let b = if *s >= 256 { 0 } else { *s as u8 };
        out.extend_from_slice(&[b, b, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in images {
        out.extend_from_slice(&img);
    }
    out
}
