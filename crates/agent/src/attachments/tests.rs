use super::*;
use hanafubuki_agent_workspace_fs::WorkspaceFsPolicy;
use std::path::PathBuf;

fn png(width: u32, height: u32) -> Vec<u8> {
    let image = DynamicImage::new_rgba8(width, height);
    let mut output = Cursor::new(Vec::new());
    image.write_to(&mut output, ImageFormat::Png).unwrap();
    output.into_inner()
}

#[test]
fn normalization_sniffs_reencodes_and_bounds_dimensions() {
    let prepared = normalize_image(
        png(3000, 2),
        "not-an-extension.bin".to_string(),
        hanafubuki_agent_kernel::AgentImageDetail::High,
    )
    .unwrap();
    assert_eq!(prepared.mime_type, "image/png");
    assert_eq!(prepared.width, 2048);
    assert!(prepared.height > 0);
    assert!(prepared.bytes.len() <= MAXIMUM_NORMALIZED_IMAGE_BYTES);
}

#[test]
fn normalization_rejects_non_images() {
    assert!(normalize_image(
        b"not an image".to_vec(),
        "bad.png".to_string(),
        hanafubuki_agent_kernel::AgentImageDetail::Auto,
    )
    .is_err());
}

#[test]
fn view_image_returns_a_high_detail_prepared_image_without_inline_tool_bytes() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "hanafubuki-view-image-{}-{stamp}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("fixture.png");
    std::fs::write(&path, png(4, 3)).unwrap();
    let files = WorkspaceFsService::new(
        WorkspaceFsPolicy::rooted_at(&root, std::iter::empty::<PathBuf>()).unwrap(),
    );
    let image = prepare_view_image(&files, "fixture.png", &CancellationToken::default()).unwrap();
    assert_eq!(image.mime_type, "image/png");
    assert_eq!((image.width, image.height), (4, 3));
    assert_eq!(image.display_name, "fixture.png");
    assert_eq!(
        image.detail,
        hanafubuki_agent_kernel::AgentImageDetail::High
    );
    std::fs::remove_dir_all(root).unwrap();
}
