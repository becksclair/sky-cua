//! Conversion of a 32-bit-per-pixel `CGImage` into an RGB8 buffer.
//!
//! ScreenCaptureKit returns SDR screenshots as 8-bit BGRA with premultiplied
//! alpha and rows padded to an alignment boundary. Premultiplied colour
//! channels are already the pixel composited over black, so dropping alpha
//! yields the correct opaque colour without a divide.

use image::RgbImage;
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::{CGBitmapInfo, CGDataProvider, CGImage, CGImageAlphaInfo};

/// In-memory order of the four 8-bit channels of one pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ChannelOrder {
    /// B, G, R, A: `kCGImageByteOrder32Little` with alpha first.
    Bgra,
    /// R, G, B, A: big-endian or default order with alpha last.
    Rgba,
    /// A, R, G, B: big-endian or default order with alpha first.
    Argb,
    /// A, B, G, R: `kCGImageByteOrder32Little` with alpha last.
    Abgr,
}

impl ChannelOrder {
    /// Derives the in-memory channel order from a `CGImage`'s byte-order and
    /// alpha-placement bits. Returns `None` for layouts that are not 8-bit
    /// four-channel packed pixels.
    pub(super) fn from_bitmap_info(byte_order: u32, alpha: CGImageAlphaInfo) -> Option<Self> {
        const ORDER_DEFAULT: u32 = 0;
        const ORDER_32_LITTLE: u32 = 2 << 12;
        const ORDER_32_BIG: u32 = 4 << 12;
        let alpha_first = match alpha {
            CGImageAlphaInfo::PremultipliedFirst
            | CGImageAlphaInfo::First
            | CGImageAlphaInfo::NoneSkipFirst => true,
            CGImageAlphaInfo::PremultipliedLast
            | CGImageAlphaInfo::Last
            | CGImageAlphaInfo::NoneSkipLast => false,
            _ => return None,
        };
        match (byte_order, alpha_first) {
            (ORDER_32_LITTLE, true) => Some(Self::Bgra),
            (ORDER_32_LITTLE, false) => Some(Self::Abgr),
            (ORDER_DEFAULT | ORDER_32_BIG, true) => Some(Self::Argb),
            (ORDER_DEFAULT | ORDER_32_BIG, false) => Some(Self::Rgba),
            _ => None,
        }
    }
}

/// Copies `image`'s pixels into a tightly packed RGB8 buffer.
pub(super) fn cg_image_to_rgb(image: &CGImage) -> Result<RgbImage, String> {
    let width = CGImage::width(Some(image));
    let height = CGImage::height(Some(image));
    let bits_per_component = CGImage::bits_per_component(Some(image));
    let bits_per_pixel = CGImage::bits_per_pixel(Some(image));
    let bytes_per_row = CGImage::bytes_per_row(Some(image));
    let bitmap_info = CGImage::bitmap_info(Some(image));
    let alpha = CGImage::alpha_info(Some(image));
    if bits_per_component != 8 || bits_per_pixel != 32 {
        return Err(format!(
            "unsupported CGImage pixel layout: {bits_per_component} bits per component, \
             {bits_per_pixel} bits per pixel"
        ));
    }
    let byte_order = (bitmap_info & CGBitmapInfo::ByteOrderInfoMask).bits();
    let order = ChannelOrder::from_bitmap_info(byte_order, alpha).ok_or_else(|| {
        format!(
            "unsupported CGImage channel layout: bitmap info {:#x}, alpha info {}",
            bitmap_info.bits(),
            alpha.0
        )
    })?;
    let provider: CFRetained<CGDataProvider> = CGImage::data_provider(Some(image))
        .ok_or_else(|| "CGImage has no data provider".to_owned())?;
    let data = CGDataProvider::data(Some(&provider))
        .ok_or_else(|| "CGDataProviderCopyData returned no pixel data".to_owned())?;
    // SAFETY: `data` is an immutable copy owned by this function and is not
    // mutated while the slice is alive.
    let bytes = unsafe { data.as_bytes_unchecked() };
    packed_to_rgb(bytes, width, height, bytes_per_row, order)
}

/// Converts row-padded 4-byte pixels into a tightly packed RGB8 image.
pub(super) fn packed_to_rgb(
    bytes: &[u8],
    width: usize,
    height: usize,
    bytes_per_row: usize,
    order: ChannelOrder,
) -> Result<RgbImage, String> {
    let row_bytes = width
        .checked_mul(4)
        .ok_or_else(|| format!("capture width {width} overflows"))?;
    if width == 0 || height == 0 {
        return Err(format!("capture image is empty ({width}x{height})"));
    }
    if bytes_per_row < row_bytes {
        return Err(format!(
            "capture row stride {bytes_per_row} is shorter than {width} pixels"
        ));
    }
    let required = bytes_per_row
        .checked_mul(height - 1)
        .and_then(|prefix| prefix.checked_add(row_bytes))
        .ok_or_else(|| format!("capture size {width}x{height} overflows"))?;
    if bytes.len() < required {
        return Err(format!(
            "capture buffer holds {} bytes; {width}x{height} at stride {bytes_per_row} needs {required}",
            bytes.len()
        ));
    }
    let out_width = u32::try_from(width).map_err(|_| format!("capture width {width} overflows"))?;
    let out_height =
        u32::try_from(height).map_err(|_| format!("capture height {height} overflows"))?;
    let mut rgb = vec![0u8; width * height * 3];
    match order {
        ChannelOrder::Bgra => convert_rows::<2, 1, 0>(bytes, &mut rgb, row_bytes, bytes_per_row),
        ChannelOrder::Rgba => convert_rows::<0, 1, 2>(bytes, &mut rgb, row_bytes, bytes_per_row),
        ChannelOrder::Argb => convert_rows::<1, 2, 3>(bytes, &mut rgb, row_bytes, bytes_per_row),
        ChannelOrder::Abgr => convert_rows::<3, 2, 1>(bytes, &mut rgb, row_bytes, bytes_per_row),
    }
    RgbImage::from_raw(out_width, out_height, rgb)
        .ok_or_else(|| "RGB buffer does not match the capture dimensions".to_owned())
}

/// Monomorphised per channel order so the inner loop has constant offsets.
fn convert_rows<const R: usize, const G: usize, const B: usize>(
    bytes: &[u8],
    rgb: &mut [u8],
    row_bytes: usize,
    bytes_per_row: usize,
) {
    let out_row_bytes = row_bytes / 4 * 3;
    for (src_row, dst_row) in bytes
        .chunks(bytes_per_row)
        .zip(rgb.chunks_exact_mut(out_row_bytes))
    {
        let (src_pixels, _) = src_row[..row_bytes].as_chunks::<4>();
        let (dst_pixels, _) = dst_row.as_chunks_mut::<3>();
        for (src, dst) in src_pixels.iter().zip(dst_pixels) {
            dst[0] = src[R];
            dst[1] = src[G];
            dst[2] = src[B];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_rows_with_padding_convert_to_rgb() {
        // 2x2 image, stride 12 bytes (4 bytes of padding per row).
        let bytes = [
            1, 2, 3, 255, 4, 5, 6, 255, 0xEE, 0xEE, 0xEE, 0xEE, //
            7, 8, 9, 255, 10, 11, 12, 255, 0xEE, 0xEE, 0xEE, 0xEE,
        ];
        let image = packed_to_rgb(&bytes, 2, 2, 12, ChannelOrder::Bgra).unwrap();
        assert_eq!(image.dimensions(), (2, 2));
        assert_eq!(image.as_raw(), &[3, 2, 1, 6, 5, 4, 9, 8, 7, 12, 11, 10]);
    }

    #[test]
    fn last_row_may_omit_trailing_padding() {
        let bytes = [1, 2, 3, 4, 0, 0, 0, 0, 5, 6, 7, 8];
        let image = packed_to_rgb(&bytes, 1, 2, 8, ChannelOrder::Rgba).unwrap();
        assert_eq!(image.as_raw(), &[1, 2, 3, 5, 6, 7]);
    }

    #[test]
    fn every_channel_order_picks_the_right_bytes() {
        let pixel = [10, 20, 30, 40];
        let cases = [
            (ChannelOrder::Bgra, [30, 20, 10]),
            (ChannelOrder::Rgba, [10, 20, 30]),
            (ChannelOrder::Argb, [20, 30, 40]),
            (ChannelOrder::Abgr, [40, 30, 20]),
        ];
        for (order, expected) in cases {
            let image = packed_to_rgb(&pixel, 1, 1, 4, order).unwrap();
            assert_eq!(image.as_raw(), &expected, "{order:?}");
        }
    }

    #[test]
    fn short_buffers_and_strides_are_rejected() {
        assert!(packed_to_rgb(&[0; 7], 1, 2, 4, ChannelOrder::Bgra).is_err());
        assert!(packed_to_rgb(&[0; 16], 2, 2, 4, ChannelOrder::Bgra).is_err());
        assert!(packed_to_rgb(&[], 0, 0, 0, ChannelOrder::Bgra).is_err());
    }

    #[test]
    fn bitmap_info_maps_to_channel_order() {
        let little = 2 << 12;
        let big = 4 << 12;
        assert_eq!(
            ChannelOrder::from_bitmap_info(little, CGImageAlphaInfo::PremultipliedFirst),
            Some(ChannelOrder::Bgra)
        );
        assert_eq!(
            ChannelOrder::from_bitmap_info(little, CGImageAlphaInfo::NoneSkipLast),
            Some(ChannelOrder::Abgr)
        );
        assert_eq!(
            ChannelOrder::from_bitmap_info(big, CGImageAlphaInfo::PremultipliedLast),
            Some(ChannelOrder::Rgba)
        );
        assert_eq!(
            ChannelOrder::from_bitmap_info(0, CGImageAlphaInfo::First),
            Some(ChannelOrder::Argb)
        );
        assert_eq!(
            ChannelOrder::from_bitmap_info(little, CGImageAlphaInfo::None),
            None
        );
        assert_eq!(
            ChannelOrder::from_bitmap_info(1 << 12, CGImageAlphaInfo::PremultipliedFirst),
            None
        );
    }
}
