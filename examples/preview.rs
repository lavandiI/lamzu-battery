//! Render the tray icon for several values into `docs/preview.png`.
//!
//! Run with `cargo run --example preview`.

use lamzu_battery::icons::{IconRenderer, battery_color, palette};
use lamzu_battery::protocol::Percent;

const SIZE: u32 = 64;
const SCALE: u32 = 4;
const PAD: u32 = 16;

fn main() {
    let labels: [(&str, u8); 5] = [("7", 7), ("42", 42), ("95", 95), ("100", 100), ("?", 0)];
    let cell = SIZE * SCALE;
    let width = PAD + (cell + PAD) * labels.len() as u32;
    let height = PAD + (cell + PAD) * 2;
    let mut canvas = vec![0_u8; (width * height * 4) as usize];

    for y in 0..height {
        for x in 0..width {
            let background = if y < height / 2 {
                [32_u8, 32, 32, 255]
            } else {
                [243_u8, 243, 243, 255]
            };
            let index = ((y * width + x) * 4) as usize;
            canvas[index..index + 4].copy_from_slice(&background);
        }
    }

    let mut renderer = IconRenderer::new();
    for (row, light) in [(0_u32, false), (1, true)] {
        for (column, (text, percent)) in labels.iter().enumerate() {
            let pal = palette(light);
            let colour = if *text == "?" {
                pal.stale
            } else {
                battery_color(Percent::new(*percent).expect("valid"), false, false, light)
            };
            let rgba = renderer.render(text, colour);
            let origin_x = PAD + column as u32 * (cell + PAD);
            let origin_y = PAD + row * (cell + PAD);
            for y in 0..SIZE {
                for x in 0..SIZE {
                    let source = ((y * SIZE + x) * 4) as usize;
                    let alpha = u32::from(rgba[source + 3]);
                    if alpha == 0 {
                        continue;
                    }
                    for dy in 0..SCALE {
                        for dx in 0..SCALE {
                            let target =
                                (((origin_y + y * SCALE + dy) * width) + origin_x + x * SCALE + dx)
                                    as usize
                                    * 4;
                            for channel in 0..3 {
                                let foreground = u32::from(rgba[source + channel]);
                                let background = u32::from(canvas[target + channel]);
                                canvas[target + channel] = u8::try_from(
                                    (foreground * alpha + background * (255 - alpha)) / 255,
                                )
                                .unwrap_or(0);
                            }
                        }
                    }
                }
            }
        }
    }

    let directory = std::path::Path::new("docs");
    std::fs::create_dir_all(directory).expect("create docs directory");
    let path = directory.join("preview.png");
    let file = std::fs::File::create(&path).expect("create preview");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("png header");
    writer.write_image_data(&canvas).expect("png data");
    println!("wrote {} ({width}x{height})", path.display());
}
