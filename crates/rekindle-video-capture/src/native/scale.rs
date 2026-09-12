//! I420 plane resampling to the encoder's fixed input resolution.
//!
//! The interim §10.6 ceiling fixes the encoded stream at 854×480; cameras
//! deliver arbitrary (usually 640×480 / 1280×720) modes, so every frame is
//! resampled to the target. Each plane is resized independently with a
//! box/area average — the standard planar-scale approach (libyuv's
//! `I420Scale`), correct for the downscale this path almost always does and
//! free of a heavyweight resize dependency. A frame already at the target
//! is returned unchanged (the common exact-mode case).

use super::convert::I420Buf;

/// Resize an I420 frame to `dst_w × dst_h` (both even).
pub(crate) fn scale_i420(src: &I420Buf, dst_w: u32, dst_h: u32) -> I420Buf {
    if src.width == dst_w && src.height == dst_h {
        return I420Buf {
            width: src.width,
            height: src.height,
            y: src.y.clone(),
            u: src.u.clone(),
            v: src.v.clone(),
        };
    }
    let y = scale_plane(&src.y, src.width, src.height, dst_w, dst_h);
    let u = scale_plane(&src.u, src.width / 2, src.height / 2, dst_w / 2, dst_h / 2);
    let v = scale_plane(&src.v, src.width / 2, src.height / 2, dst_w / 2, dst_h / 2);
    I420Buf {
        width: dst_w,
        height: dst_h,
        y,
        u,
        v,
    }
}

/// Box/area-average resample of one single-channel `u8` plane.
fn scale_plane(src: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<u8> {
    let mut out = vec![0u8; dst_w as usize * dst_h as usize];
    for dy in 0..dst_h {
        let sy0 = dy * src_h / dst_h;
        let sy1 = ((dy + 1) * src_h / dst_h).max(sy0 + 1).min(src_h);
        for dx in 0..dst_w {
            let sx0 = dx * src_w / dst_w;
            let sx1 = ((dx + 1) * src_w / dst_w).max(sx0 + 1).min(src_w);
            let mut sum = 0u32;
            let mut count = 0u32;
            for sy in sy0..sy1 {
                let row = sy as usize * src_w as usize;
                for sx in sx0..sx1 {
                    sum += u32::from(src[row + sx as usize]);
                    count += 1;
                }
            }
            out[dy as usize * dst_w as usize + dx as usize] =
                u8::try_from(sum / count.max(1)).unwrap_or(255);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u32, height: u32, y: u8, u: u8, v: u8) -> I420Buf {
        I420Buf {
            width,
            height,
            y: vec![y; (width * height) as usize],
            u: vec![u; ((width / 2) * (height / 2)) as usize],
            v: vec![v; ((width / 2) * (height / 2)) as usize],
        }
    }

    #[test]
    fn passthrough_when_target_matches() {
        let f = solid(4, 4, 100, 50, 200);
        let out = scale_i420(&f, 4, 4);
        assert_eq!(out.y, f.y);
        assert_eq!(out.u.len(), 4);
    }

    #[test]
    fn downscale_halves_dims_and_preserves_a_solid_field() {
        let f = solid(8, 8, 130, 60, 190);
        let out = scale_i420(&f, 4, 4);
        assert_eq!((out.width, out.height), (4, 4));
        assert_eq!(out.y.len(), 16);
        assert_eq!(out.u.len(), 4);
        assert_eq!(out.v.len(), 4);
        // Averaging a solid plane leaves it solid.
        assert!(out.y.iter().all(|&p| p == 130));
        assert!(out.u.iter().all(|&p| p == 60));
        assert!(out.v.iter().all(|&p| p == 190));
    }

    #[test]
    fn upscale_produces_target_sizes() {
        let f = solid(4, 4, 77, 88, 99);
        let out = scale_i420(&f, 8, 8);
        assert_eq!(out.y.len(), 64);
        assert_eq!(out.u.len(), 16);
        assert!(out.y.iter().all(|&p| p == 77));
    }
}
