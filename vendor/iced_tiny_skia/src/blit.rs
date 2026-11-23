//! A copy of a premultiplied pixmap over the pixels at a whole pixel offset,
//! with the `SourceOver` blend, an opacity and a clip mask.
//!
//! It draws what `tiny_skia::PixmapMut::draw_pixmap` draws with the identity
//! transform, but in 8-bit integers: tiny-skia sends a pixmap through the
//! pattern shader of its floating point pipeline, which costs many times more
//! for each pixel of each glyph, icon and image of each frame.

/// Draws `src` with its top left corner at `x`, `y` of `dst`.
pub fn draw(
    dst: &mut tiny_skia::PixmapMut<'_>,
    x: i32,
    y: i32,
    src: tiny_skia::PixmapRef<'_>,
    opacity: f32,
    clip_mask: Option<&tiny_skia::Mask>,
) {
    let opacity = (opacity.clamp(0.0, 1.0) * 255.0).round() as u32;
    if opacity == 0 {
        return;
    }

    let (dw, dh) = (i64::from(dst.width()), i64::from(dst.height()));
    let (sw, sh) = (i64::from(src.width()), i64::from(src.height()));
    let (x0, y0) = (i64::from(x).max(0), i64::from(y).max(0));
    let (x1, y1) = ((i64::from(x) + sw).min(dw), (i64::from(y) + sh).min(dh));
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    if clip_mask.is_some_and(|m| {
        i64::from(m.width()) != dw || i64::from(m.height()) != dh
    }) {
        return;
    }

    let mask = clip_mask.map(tiny_skia::Mask::data);
    let src_data = src.data();
    let dst_width = dw as usize;
    let dst_data = dst.data_mut();
    let row = (x1 - x0) as usize;

    for py in y0..y1 {
        let s = (((py - i64::from(y)) * sw + (x0 - i64::from(x))) * 4) as usize;
        let d = (py as usize * dst_width + x0 as usize) * 4;
        let src_row = &src_data[s..s + row * 4];
        let dst_row = &mut dst_data[d..d + row * 4];
        let pixels = src_row.chunks_exact(4).zip(dst_row.chunks_exact_mut(4));
        match mask {
            None => {
                for (s, d) in pixels {
                    over(s, d, opacity);
                }
            }
            Some(mask) => {
                let m = py as usize * dst_width + x0 as usize;
                for ((s, d), &c) in pixels.zip(&mask[m..m + row]) {
                    over(s, d, div255(opacity * u32::from(c)));
                }
            }
        }
    }
}

/// `s` over `d`, with `s` scaled by `coverage` out of 255.
#[inline(always)]
fn over(s: &[u8], d: &mut [u8], coverage: u32) {
    let a = div255(u32::from(s[3]) * coverage);
    if a == 0 {
        return;
    }
    let keep = 255 - a;
    for i in 0..4 {
        let s = match coverage {
            255 => u32::from(s[i]),
            _ => div255(u32::from(s[i]) * coverage),
        };
        d[i] = (s + div255(u32::from(d[i]) * keep)).min(255) as u8;
    }
}

/// `v / 255`, rounded, for `v` up to 255 * 255.
#[inline(always)]
fn div255(v: u32) -> u32 {
    let v = v + 128;
    (v + (v >> 8)) >> 8
}

#[cfg(test)]
mod tests {
    /// The copy draws what tiny-skia draws, within one step of each channel.
    #[test]
    fn draws_like_tiny_skia() {
        let mut src = tiny_skia::Pixmap::new(7, 5).unwrap();
        for (i, p) in src.data_mut().chunks_exact_mut(4).enumerate() {
            let a = (i * 37 % 256) as u8;
            let c = |k: usize| ((i * k) % 256 * a as usize / 255) as u8;
            p.copy_from_slice(&[c(11), c(23), c(5), a]);
        }
        let mut mask = tiny_skia::Mask::new(9, 8).unwrap();
        for (i, m) in mask.data_mut().iter_mut().enumerate() {
            *m = (i * 53 % 256) as u8;
        }
        let cases = [
            (1, 2, 1.0, false),
            (-2, 4, 1.0, false),
            (1, 2, 0.6, false),
            (3, -1, 0.8, true),
            (0, 0, 1.0, true),
        ];
        for (x, y, opacity, masked) in cases {
            let mut a = tiny_skia::Pixmap::new(9, 8).unwrap();
            for (i, p) in a.data_mut().chunks_exact_mut(4).enumerate() {
                let al = (i * 91 % 256) as u8;
                let c = |k: usize| ((i * k) % 256 * al as usize / 255) as u8;
                p.copy_from_slice(&[c(7), c(3), al / 2, al]);
            }
            let (mut b, before) = (a.clone(), a.clone());
            let mask = masked.then_some(&mask);
            let paint = tiny_skia::PixmapPaint {
                opacity,
                ..Default::default()
            };
            let identity = tiny_skia::Transform::identity();
            a.draw_pixmap(x, y, src.as_ref(), &paint, identity, mask);
            super::draw(&mut b.as_mut(), x, y, src.as_ref(), opacity, mask);
            let pixels = a.data().iter().zip(b.data()).zip(before.data());
            for (i, ((p, q), o)) in pixels.enumerate() {
                let (px, py) = ((i / 4 % 9) as i32, (i / 36) as i32);
                // tiny-skia pads one more column at a negative `x`.
                let inside =
                    (x..x + 7).contains(&px) && (y..y + 5).contains(&py);
                if inside {
                    assert!(p.abs_diff(*q) <= 1, "{x} {y}: {px} {py}: {p} {q}");
                } else {
                    assert_eq!(q, o, "{x} {y}: {px} {py} is outside");
                }
            }
        }
    }
}
