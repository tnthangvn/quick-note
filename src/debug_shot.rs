//! Debug-only: `QUICK_NOTE_SCREENSHOT=/path/out.ppm` captures one frame and exits.
//! Used to eyeball the UI on machines without a screenshot tool.

use std::io::Write;

const WARMUP_FRAMES: u64 = 20;

pub fn tick(ctx: &egui::Context) {
    let Ok(path) = std::env::var("QUICK_NOTE_SCREENSHOT") else {
        return;
    };
    let frame = ctx.cumulative_frame_nr();
    if frame == WARMUP_FRAMES {
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
    }
    ctx.request_repaint();
    let shot = ctx.input(|i| {
        i.raw.events.iter().find_map(|e| match e {
            egui::Event::Screenshot { image, .. } => Some(image.clone()),
            _ => None,
        })
    });
    if let Some(image) = shot {
        if let Err(e) = write_ppm(&path, &image) {
            eprintln!("screenshot failed: {e}");
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

fn write_ppm(path: &str, image: &egui::ColorImage) -> std::io::Result<()> {
    let [w, h] = image.size;
    let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
    write!(file, "P6\n{w} {h}\n255\n")?;
    for px in &image.pixels {
        file.write_all(&[px.r(), px.g(), px.b()])?;
    }
    Ok(())
}
