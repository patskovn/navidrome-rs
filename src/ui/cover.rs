//! Album artwork, drawn as colour half-blocks.
//!
//! A terminal cannot show a JPEG without a graphics protocol, so the image is
//! resampled onto the character grid instead: each cell draws `▀` with the
//! foreground set to its upper pixel and the background to its lower one. One
//! cell therefore carries two real pixels, and the result keeps full colour —
//! unlike an ASCII or braille ramp, which encodes intensity in the character
//! and throws colour away.

use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::widgets::Widget;

/// Edge length the artwork is normalised to on decode. Small enough to keep in
/// state cheaply, large enough to oversample any panel we draw into.
const SIZE: u32 = 64;

/// A decoded, downscaled cover. Plain data, so it is safe to hold in reducer
/// state — no interior mutability, cheap to compare and clone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cover {
    width: u32,
    height: u32,
    pixels: Vec<[u8; 3]>,
}

impl Cover {
    /// Decodes and downscales encoded image bytes.
    pub fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        let image = image::load_from_memory(bytes)?
            .resize_exact(SIZE, SIZE, image::imageops::FilterType::Triangle)
            .to_rgb8();

        Ok(Self {
            width: image.width(),
            height: image.height(),
            pixels: image.pixels().map(|p| [p[0], p[1], p[2]]).collect(),
        })
    }

    fn sample(&self, x: u32, y: u32) -> Color {
        let x = x.min(self.width.saturating_sub(1));
        let y = y.min(self.height.saturating_sub(1));
        let index = (y * self.width + x) as usize;
        match self.pixels.get(index) {
            Some([r, g, b]) => Color::Rgb(*r, *g, *b),
            None => Color::Reset,
        }
    }
}

/// Draws a [`Cover`] into a rectangle, letterboxed to stay square.
pub struct CoverWidget<'a> {
    cover: &'a Cover,
}

impl<'a> CoverWidget<'a> {
    pub fn new(cover: &'a Cover) -> Self {
        Self { cover }
    }
}

impl Widget for CoverWidget<'_> {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        // A cell is roughly twice as tall as it is wide, and half-blocks split
        // it again vertically — so one *pixel* is about square, and a square
        // image wants `cols == rows * 2`. Fit that inside the area and centre
        // it, rather than stretching.
        let rows = area.height.min(area.width.div_ceil(2));
        let cols = (rows * 2).min(area.width);
        if rows == 0 || cols == 0 {
            return;
        }
        let x0 = area.x + (area.width - cols) / 2;
        let y0 = area.y + (area.height - rows) / 2;

        let pixel_rows = u32::from(rows) * 2;
        for row in 0..rows {
            for col in 0..cols {
                // Two stacked pixels share one cell.
                let sx = u32::from(col) * self.cover.width / u32::from(cols);
                let upper = u32::from(row) * 2 * self.cover.height / pixel_rows;
                let lower = (u32::from(row) * 2 + 1) * self.cover.height / pixel_rows;

                if let Some(cell) = buf.cell_mut((x0 + col, y0 + row)) {
                    cell.set_char('▀');
                    cell.set_style(
                        Style::new()
                            .fg(self.cover.sample(sx, upper))
                            .bg(self.cover.sample(sx, lower)),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use ratatui::buffer::Buffer;

    /// A 2x2 image: red, green / blue, white.
    fn fixture() -> Vec<u8> {
        let mut img = image::RgbImage::new(2, 2);
        img.put_pixel(0, 0, image::Rgb([255, 0, 0]));
        img.put_pixel(1, 0, image::Rgb([0, 255, 0]));
        img.put_pixel(0, 1, image::Rgb([0, 0, 255]));
        img.put_pixel(1, 1, image::Rgb([255, 255, 255]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }

    #[test]
    fn decodes_and_normalises_to_a_fixed_size() {
        let cover = Cover::decode(&fixture()).unwrap();
        assert_eq!(cover.width, SIZE);
        assert_eq!(cover.height, SIZE);
        assert_eq!(cover.pixels.len() as u32, SIZE * SIZE);
    }

    #[test]
    fn rejects_bytes_that_are_not_an_image() {
        assert!(Cover::decode(b"definitely not a png").is_err());
    }

    /// The whole point: cells must carry colour, unlike the visualiser.
    #[test]
    fn every_drawn_cell_carries_a_foreground_and_background_colour() {
        let cover = Cover::decode(&fixture()).unwrap();
        let area = Rect::new(0, 0, 16, 8);
        let mut buf = Buffer::empty(area);
        CoverWidget::new(&cover).render(area, &mut buf);

        let centre = buf.cell((8, 4)).unwrap();
        assert_eq!(centre.symbol(), "▀");
        assert!(
            matches!(centre.style().fg, Some(Color::Rgb(..))),
            "upper pixel should set an RGB foreground"
        );
        assert!(
            matches!(centre.style().bg, Some(Color::Rgb(..))),
            "lower pixel should set an RGB background"
        );
    }

    /// Covers are square; a wide area must letterbox rather than stretch.
    #[test]
    fn a_wide_area_is_letterboxed_not_stretched() {
        let cover = Cover::decode(&fixture()).unwrap();
        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);
        CoverWidget::new(&cover).render(area, &mut buf);

        // rows = 6, so cols = 12, centred: columns 14..26 are drawn.
        assert_eq!(
            buf.cell((20, 3)).unwrap().symbol(),
            "▀",
            "centre should be drawn"
        );
        assert_eq!(
            buf.cell((1, 3)).unwrap().symbol(),
            " ",
            "left should be blank"
        );
        assert_eq!(
            buf.cell((38, 3)).unwrap().symbol(),
            " ",
            "right should be blank"
        );
    }

    #[test]
    fn a_tiny_area_is_ignored_rather_than_panicking() {
        let cover = Cover::decode(&fixture()).unwrap();
        let area = Rect::new(0, 0, 1, 0);
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        CoverWidget::new(&cover).render(area, &mut buf);
    }
}

#[cfg(test)]
mod preview {
    use super::*;
    use ratatui::buffer::Buffer;

    /// `cargo test cover_preview -- --nocapture --ignored`
    #[test]
    #[ignore = "visual check only"]
    fn cover_preview() {
        let mut img = image::RgbImage::new(64, 64);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = image::Rgb([(x * 4) as u8, (y * 4) as u8, 128]);
        }
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();

        let cover = Cover::decode(&bytes).unwrap();
        let area = Rect::new(0, 0, 24, 12);
        let mut buf = Buffer::empty(area);
        CoverWidget::new(&cover).render(area, &mut buf);

        println!();
        for y in 0..area.height {
            let mut line = String::new();
            for x in 0..area.width {
                let cell = buf.cell((x, y)).unwrap();
                if let (Some(Color::Rgb(fr, fg, fb)), Some(Color::Rgb(br, bg, bb))) =
                    (cell.style().fg, cell.style().bg)
                {
                    line.push_str(&format!(
                        "\x1b[38;2;{fr};{fg};{fb}m\x1b[48;2;{br};{bg};{bb}m{}",
                        cell.symbol()
                    ));
                } else {
                    line.push(' ');
                }
            }
            println!("{line}\x1b[0m");
        }
        println!();
    }
}
