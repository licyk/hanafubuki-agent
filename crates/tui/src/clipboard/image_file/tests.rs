use std::io::Cursor;

use image::{ImageFormat, Rgba, RgbaImage};

use super::*;

fn encoded(format: ImageFormat) -> Vec<u8> {
    let image = RgbaImage::from_pixel(3, 2, Rgba([10, 20, 30, 255]));
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .to_rgb8()
        .write_to(&mut bytes, format)
        .unwrap();
    bytes.into_inner()
}

#[test]
fn png_jpeg_gif_and_webp_images_can_be_attached() {
    let directory = tempfile::tempdir().unwrap();
    for (name, format) in [
        ("a.png", ImageFormat::Png),
        ("b.jpg", ImageFormat::Jpeg),
        ("c.gif", ImageFormat::Gif),
        ("d.webp", ImageFormat::WebP),
        // The content decides, not the name.
        ("e.txt", ImageFormat::Png),
    ] {
        let path = directory.path().join(name);
        std::fs::write(&path, encoded(format)).unwrap();
        assert_eq!(inspect(&path), Inspection::Image, "{name}");
        assert!(is_image(&path), "{name}");
    }
}

#[test]
fn what_is_not_an_image_file_is_not_one() {
    let directory = tempfile::tempdir().unwrap();
    let text = directory.path().join("notes.png");
    std::fs::write(&text, "just some text").unwrap();
    assert_eq!(inspect(&text), Inspection::NotAnImage);
    assert!(!is_image(&text));
    assert_eq!(inspect(directory.path()), Inspection::NotAnImage);
    assert_eq!(
        inspect(&directory.path().join("missing.png")),
        Inspection::NotAnImage
    );
}

#[test]
fn an_image_the_agent_does_not_send_is_refused_with_why() {
    let directory = tempfile::tempdir().unwrap();
    // A bitmap's header: an image, but not one of the four formats.
    let bitmap = directory.path().join("scan.bmp");
    let mut bytes = b"BM".to_vec();
    bytes.resize(64, 0);
    std::fs::write(&bitmap, bytes).unwrap();
    assert_eq!(
        inspect(&bitmap),
        Inspection::Refused(
            "scan.bmp is not a PNG, JPEG, GIF or WebP image, the formats a model is sent."
                .to_string()
        )
    );

    // Larger than the Agent reads: refused before it is decoded.
    let large = directory.path().join("large.png");
    let file = std::fs::File::create(&large).unwrap();
    std::io::Write::write_all(&mut &file, &encoded(ImageFormat::Png)).unwrap();
    file.set_len(MAXIMUM_SOURCE_IMAGE_BYTES as u64 + 1024 * 1024)
        .unwrap();
    drop(file);
    assert_eq!(
        inspect(&large),
        Inspection::Refused(
            "large.png is too large to attach: 21.0 MiB, at most 20.0 MiB.".to_string()
        )
    );

    // A PNG whose header cannot be read.
    let broken = directory.path().join("broken.png");
    std::fs::write(&broken, &encoded(ImageFormat::Png)[..12]).unwrap();
    assert!(matches!(
        inspect(&broken),
        Inspection::Refused(why) if why.starts_with("broken.png cannot be read as an image: ")
    ));
}

#[test]
fn too_many_pixels_or_none_are_refused() {
    assert_eq!(check_pixels(2048, 2048), Ok(()));
    assert_eq!(
        check_pixels(8000, 6000),
        Err("is too large to attach: 8000×6000 pixels, at most 40 million.".to_string())
    );
    assert_eq!(
        check_pixels(0, 10),
        Err("is empty (no pixels).".to_string())
    );
    assert_eq!(
        encode_png(&RgbaImage::new(0, 0)),
        Err("The clipboard's image is empty (no pixels).".to_string())
    );
}

#[test]
fn a_clipboard_image_is_encoded_and_saved_into_the_runs_directory() {
    let image = RgbaImage::from_pixel(4, 3, Rgba([200, 100, 50, 255]));
    let png = encode_png(&image).unwrap();
    assert_eq!(
        image::guess_format(&png).unwrap(),
        ImageFormat::Png,
        "encoded as PNG"
    );
    let mut directory = ImageDirectory::default();
    let first = directory.save(&png).unwrap();
    let second = directory.save(&png).unwrap();
    assert_eq!(file_name(&first), "clipboard-1.png");
    assert_eq!(file_name(&second), "clipboard-2.png");
    assert_eq!(first.parent(), second.parent());
    let saved = image::open(&first).unwrap().to_rgba8();
    assert_eq!(saved, image);
    // The run's end removes the directory and what it holds.
    let parent = first.parent().unwrap().to_path_buf();
    drop(directory);
    assert!(!parent.exists());
}

#[test]
fn a_persons_file_is_attached_by_its_absolute_path() {
    let image = file_image(Path::new("pictures/cat.png"));
    assert_eq!(image.name, "cat.png");
    let ImageSource::File(path) = image.source else {
        panic!("a file of the person's");
    };
    assert!(path.is_absolute());
    assert!(path.ends_with("pictures/cat.png"));
}
