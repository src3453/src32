//! Shared pixel compositing primitives for all VDP graphics providers.

pub type Rgba = [u8; 4];

/// Straight-alpha source-over blend with round-to-nearest integer arithmetic.
pub fn source_over(src: Rgba, dst: Rgba) -> Rgba {
    if src[3] == 0 {
        return dst;
    }
    if src[3] == 255 || dst[3] == 0 {
        return src;
    }

    let a = src[3] as u64;
    let dst_a = dst[3] as u64;
    let inv = 255 - a;
    let out_a = a + (dst_a * inv + 127) / 255;
    let mut out = [0; 4];
    for channel in 0..3 {
        if out_a != 0 {
            let premultiplied = src[channel] as u64 * a * 255 + dst[channel] as u64 * dst_a * inv;
            out[channel] = ((premultiplied + out_a * 127) / (out_a * 255)) as u8;
        }
    }
    out[3] = out_a.min(255) as u8;
    out
}

/// Composes one pixel from GP0 (back) through GP7 (front).
pub fn compose_gp(layers: [Rgba; 8], backdrop: Rgba) -> Rgba {
    layers
        .into_iter()
        .filter(|src| src[3] != 0)
        .fold(backdrop, |dst, src| source_over(src, dst))
}
