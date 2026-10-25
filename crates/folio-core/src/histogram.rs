//! A photo's histogram: how many pixels sit at each level, per channel and
//! for brightness. Pure: pixels in, counts and drawable curves out. Worked
//! out from the thumbnail, which has plenty of pixels for its shape.

use image::RgbaImage;

pub const LEVELS: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Histogram {
    pub red: [u32; LEVELS],
    pub green: [u32; LEVELS],
    pub blue: [u32; LEVELS],
    /// Brightness, weighted the way eyes see it (Rec. 709).
    pub luma: [u32; LEVELS],
    pixels: u32,
}

impl Histogram {
    pub fn of(image: &RgbaImage) -> Self {
        let mut histogram = Self { red: [0; LEVELS], green: [0; LEVELS], blue: [0; LEVELS], luma: [0; LEVELS], pixels: 0 };
        for pixel in image.pixels() {
            let [r, g, b, _] = pixel.0;
            histogram.red[usize::from(r)] += 1;
            histogram.green[usize::from(g)] += 1;
            histogram.blue[usize::from(b)] += 1;
            let luma = (2126 * u32::from(r) + 7152 * u32::from(g) + 722 * u32::from(b)) / 10_000;
            histogram.luma[luma as usize] += 1;
            histogram.pixels += 1;
        }
        histogram
    }

    /// Shares of pixels with a channel at pure black and at pure white:
    /// lost shadows and blown highlights.
    pub fn clipped(&self) -> (f32, f32) {
        let share = |count: u32| if self.pixels == 0 { 0. } else { count as f32 / self.pixels as f32 };
        let at = |level: usize| self.red[level].max(self.green[level]).max(self.blue[level]);
        (share(at(0)), share(at(LEVELS - 1)))
    }

    /// The channels squeezed into `bins` heights from 0 to 1, in the order
    /// red, green, blue, brightness, ready to draw. All four share one
    /// scale. The first and last level are left out of the scale, so a
    /// clipped sky doesn't flatten everything else.
    pub fn curves(&self, bins: usize) -> [Vec<f32>; 4] {
        let bins = bins.clamp(1, LEVELS);
        let squeeze = |levels: &[u32; LEVELS]| -> Vec<u32> {
            (0..bins).map(|bin| levels[bin * LEVELS / bins..(bin + 1) * LEVELS / bins].iter().sum()).collect()
        };
        let channels = [squeeze(&self.red), squeeze(&self.green), squeeze(&self.blue), squeeze(&self.luma)];
        let inner = 1..bins.saturating_sub(1).max(1);
        let tallest = channels.iter().flat_map(|c| c[inner.clone()].iter().copied()).max().unwrap_or(0).max(1) as f32;
        channels.map(|c| c.iter().map(|&count| (count as f32 / tallest).min(1.)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_levels_per_channel_and_brightness() {
        let mut image = RgbaImage::from_pixel(4, 1, image::Rgba([255, 0, 0, 255]));
        image.put_pixel(3, 0, image::Rgba([128, 128, 128, 255]));
        let histogram = Histogram::of(&image);
        assert_eq!(histogram.red[255], 3);
        assert_eq!(histogram.green[0], 3);
        assert_eq!(histogram.luma[54], 3, "pure red is dark to the eye");
        assert_eq!(histogram.luma[128], 1);
        assert_eq!(histogram.clipped(), (0.75, 0.75));
    }

    #[test]
    fn curves_share_a_scale_that_ignores_clipped_ends() {
        let mut image = RgbaImage::from_pixel(10, 10, image::Rgba([255, 255, 255, 255]));
        image.put_pixel(0, 0, image::Rgba([100, 100, 100, 255]));
        image.put_pixel(1, 0, image::Rgba([100, 100, 100, 255]));
        let [red, _, _, luma] = Histogram::of(&image).curves(64);
        assert_eq!(red.len(), 64);
        assert_eq!(red[25], 1., "the mid-tone bump sets the scale");
        assert_eq!(luma[63], 1., "the clipped end is capped, not the scale");
    }
}
