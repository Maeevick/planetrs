//! Rasterising an SVG, without a browser.
//!
//! The spike shelled out to a Chromium that a private repository happened to have installed, which
//! is not something a tool anyone else can run may do. This is the same picture through `resvg`, in
//! process, with no network and no Node.
//!
//! **A PNG is not the reproducible artefact.** The GeoJSON and the SVG are byte-deterministic given
//! the same harvest; a raster of text is not, because the glyphs come from whatever fonts the
//! machine has. That is equally true of a browser. When a run has to be defensible afterwards, keep
//! the SVG and the harvest, and treat the PNG as a picture of them.

use std::path::Path;

use resvg::tiny_skia;
use resvg::usvg;

use crate::error::ClipError;

/// The SVG drawn at `scale` times its stated size, as PNG bytes.
pub fn render(svg: &str, scale: f32) -> Result<Vec<u8>, ClipError> {
    let mut options = usvg::Options::default();
    options.fontdb_mut().load_system_fonts();

    let tree = usvg::Tree::from_str(svg, &options).map_err(|error| ClipError::Raster {
        detail: error.to_string(),
    })?;

    let size = tree
        .size()
        .to_int_size()
        .scale_by(scale)
        .ok_or(ClipError::Raster {
            detail: format!("a scale of {scale} leaves no pixels"),
        })?;

    let mut pixmap = tiny_skia::Pixmap::new(size.width(), size.height()).ok_or(ClipError::Raster {
        detail: format!("{} by {} is too large to allocate", size.width(), size.height()),
    })?;

    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    pixmap.encode_png().map_err(|error| ClipError::Raster {
        detail: error.to_string(),
    })
}

pub fn write(path: &Path, svg: &str, scale: f32) -> Result<u64, ClipError> {
    let bytes = render(svg, scale)?;
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|error| ClipError::writing(path, error))?;
    }
    std::fs::write(path, &bytes).map_err(|error| ClipError::writing(path, error))?;
    Ok(bytes.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 10 10\" \
                          width=\"10\" height=\"10\"><rect width=\"10\" height=\"10\" \
                          fill=\"#ff0000\"/></svg>";

    #[test]
    fn when_an_svg_is_rasterised_then_the_bytes_are_a_png() {
        let bytes = render(SQUARE, 1.0).unwrap();

        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }

    /// A hero image is drawn at twice its stated size so it stays sharp on a dense display.
    #[test]
    fn when_a_scale_is_given_then_the_raster_is_that_much_larger() {
        let once = render(SQUARE, 1.0).unwrap();
        let twice = render(SQUARE, 2.0).unwrap();

        assert!(twice.len() > once.len());
    }

    #[test]
    fn when_the_svg_is_not_an_svg_then_it_is_refused_rather_than_drawn_blank() {
        assert!(render("a map, but described in words", 1.0).is_err());
    }
}
