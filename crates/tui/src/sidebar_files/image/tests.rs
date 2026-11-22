use super::*;
use crate::{custom_terminal::Terminal, test_support::VT100Backend};

#[test]
fn native_images_are_transmitted_once_then_erased_on_replacement_and_close() {
    let raster = RgbaImage::from_pixel(5, 6, Rgba([220, 90, 30, 255]));
    let area = Rect::new(3, 1, 2, 1);
    let kitty = ImagePlacement {
        area,
        id: 319,
        encoded: kitty(&raster, area, 319).into(),
        protocol: Protocol::Kitty,
    };
    let sixel = ImagePlacement {
        area,
        id: 320,
        encoded: sixel(&raster).into(),
        protocol: Protocol::Sixel,
    };
    let mut term = Terminal::with_options(VT100Backend::new(24, 8)).unwrap();
    term.set_viewport_area(Rect::new(0, 0, 24, 8));
    let mut show = |image| {
        term.draw(|frame| frame.sidebar_image = image).unwrap();
        term.backend_mut().take_written()
    };
    let first = show(Some(kitty.clone()));
    assert!(first.contains("a=T,f=32,s=5,v=6,c=2,r=1,i=319"));
    let unchanged = show(Some(kitty));
    assert!(!unchanged.contains("\x1b_G"));
    assert!(
        unchanged.contains("\x1b[2;6H"),
        "clear to end starts after the image on row two: {unchanged:?}"
    );
    let replacement = show(Some(sixel));
    assert!(replacement.contains("a=d,d=I,i=319"));
    assert!(replacement.contains("\x1bP9;1;0q\"1;1;5;6"));
    let closed = show(None);
    assert!(closed.contains("\x1b[2X"));
}

#[test]
fn kitty_chunks_are_individually_terminated_and_only_last_chunk_ends_the_transfer() {
    let image = RgbaImage::new(64, 64);
    let bytes = kitty(&image, Rect::new(1, 1, 8, 4), 39);
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(text.contains(",m=1;"));
    assert!(text.contains("\x1b_Gm=0;"));
    assert!(text.ends_with("\x1b\\"));
}
