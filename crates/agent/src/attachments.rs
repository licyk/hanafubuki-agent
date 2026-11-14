use std::io::Cursor;

use base64::Engine as _;
use image::{DynamicImage, GenericImageView, ImageDecoder, ImageFormat, ImageReader};

use super::*;

pub const MAXIMUM_SOURCE_IMAGE_BYTES: usize = 20 * 1024 * 1024;
pub const MAXIMUM_SOURCE_IMAGES_BYTES: usize = 64 * 1024 * 1024;
pub const MAXIMUM_IMAGE_PIXELS: u64 = 40_000_000;
pub const MAXIMUM_IMAGE_DIMENSION: u32 = 2048;
pub const MAXIMUM_NORMALIZED_IMAGE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DesktopAgentImageInput {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub attachment: Option<hanafubuki_agent_kernel::AgentImageAttachmentRef>,
    pub display_name: String,
    #[serde(default)]
    pub detail: hanafubuki_agent_kernel::AgentImageDetail,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentImagePreview {
    pub data_url: String,
    pub width: u32,
    pub height: u32,
}

impl AgentApplicationService {
    /// Normalizes clipboard bytes and stages them through the history authority.
    pub fn import_image_bytes(
        &self,
        session_id: AgentSessionId,
        bytes: Vec<u8>,
        display_name: String,
        detail: hanafubuki_agent_kernel::AgentImageDetail,
    ) -> Result<hanafubuki_agent_kernel::AgentImageAttachmentRef, String> {
        if bytes.len() > MAXIMUM_SOURCE_IMAGE_BYTES {
            return Err("image exceeds source byte limit".to_string());
        }
        if display_name.is_empty()
            || display_name.len() > 256
            || display_name.as_bytes().contains(&0)
        {
            return Err("image display name must contain 1 to 256 bytes without NUL".to_string());
        }
        self.core
            .activate_session(session_id)
            .map_err(|error| error.to_string())?;
        let prepared = normalize_image(bytes, display_name, detail)?;
        self.core
            .stage_image_attachment(session_id, prepared)
            .map_err(|error| error.to_string())
    }

    pub fn preview_selected_image(&self, path: String) -> Result<AgentImagePreview, String> {
        let selected = self
            .files
            .read_selected_binary(
                &path,
                MAXIMUM_SOURCE_IMAGE_BYTES,
                &CancellationToken::default(),
            )
            .map_err(|error| format!("read selected image preview: {error}"))?;
        let prepared = normalize_image(
            selected.bytes,
            "preview".to_string(),
            hanafubuki_agent_kernel::AgentImageDetail::Low,
        )?;
        bounded_preview(&prepared.bytes)
    }

    pub fn read_image_preview(
        &self,
        session_id: AgentSessionId,
        attachment: hanafubuki_agent_kernel::AgentImageAttachmentRef,
    ) -> Result<AgentImagePreview, String> {
        let bytes = self
            .core
            .read_input_image_attachment(session_id, &attachment)
            .map_err(|error| error.to_string())?;
        bounded_preview(&bytes)
    }
}

/// Reads a workspace image for `view_image` and normalizes it at high detail.
pub(crate) fn prepare_view_image(
    files: &WorkspaceFsService,
    path: &str,
    cancellation: &CancellationToken,
) -> Result<hanafubuki_agent_kernel::PreparedAgentImage, String> {
    let selected = files
        .read_bounded_binary(path, MAXIMUM_SOURCE_IMAGE_BYTES, cancellation)
        .map_err(|error| format!("read image: {error}"))?;
    let display_name = Path::new(&selected.canonical_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("image")
        .to_string();
    normalize_image(
        selected.bytes,
        display_name,
        hanafubuki_agent_kernel::AgentImageDetail::High,
    )
}

fn bounded_preview(bytes: &[u8]) -> Result<AgentImagePreview, String> {
    let image = image::load_from_memory(bytes)
        .map_err(|error| format!("decode stored image preview: {error}"))?
        .thumbnail(512, 512);
    let (width, height) = image.dimensions();
    let mut encoded = Cursor::new(Vec::new());
    image
        .write_to(&mut encoded, ImageFormat::Png)
        .map_err(|error| format!("encode stored image preview: {error}"))?;
    let encoded = encoded.into_inner();
    if encoded.len() > 1024 * 1024 {
        return Err("image preview exceeds 1 MiB".to_string());
    }
    Ok(AgentImagePreview {
        data_url: format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(encoded)
        ),
        width,
        height,
    })
}

pub(crate) fn prepare_and_stage_images(
    core: &AgentService,
    files: &WorkspaceFsService,
    session_id: AgentSessionId,
    inputs: Vec<DesktopAgentImageInput>,
) -> Result<Vec<hanafubuki_agent_kernel::AgentImageAttachmentRef>, String> {
    // Checked before any image is read or staged.
    if inputs.len() > hanafubuki_agent_kernel::MAXIMUM_USER_IMAGES_PER_MESSAGE {
        return Err(format!(
            "at most {} images may be submitted",
            hanafubuki_agent_kernel::MAXIMUM_USER_IMAGES_PER_MESSAGE
        ));
    }
    let cancellation = CancellationToken::default();
    let mut source_total = 0usize;
    let mut staged = Vec::with_capacity(inputs.len());
    for input in inputs {
        if input.display_name.is_empty()
            || input.display_name.len() > 256
            || input.display_name.as_bytes().contains(&0)
        {
            return Err("image display name must contain 1 to 256 bytes without NUL".to_string());
        }
        if let (None, Some(attachment)) = (&input.path, &input.attachment) {
            core.read_input_image_attachment(session_id, attachment)
                .map_err(|error| error.to_string())?;
            staged.push(attachment.clone());
            continue;
        }
        let path = match (&input.path, &input.attachment) {
            (Some(path), None) => path,
            _ => {
                return Err(
                    "image input must contain exactly one selected path or durable attachment"
                        .to_string(),
                )
            }
        };
        let selected = files
            .read_selected_binary(path, MAXIMUM_SOURCE_IMAGE_BYTES, &cancellation)
            .map_err(|error| format!("read selected image: {error}"))?;
        source_total = source_total
            .checked_add(selected.bytes.len())
            .ok_or_else(|| "selected image byte total overflowed".to_string())?;
        if source_total > MAXIMUM_SOURCE_IMAGES_BYTES {
            return Err(format!(
                "selected images exceed {MAXIMUM_SOURCE_IMAGES_BYTES} source bytes"
            ));
        }
        let prepared = normalize_image(selected.bytes, input.display_name, input.detail)?;
        staged.push(
            core.stage_image_attachment(session_id, prepared)
                .map_err(|error| error.to_string())?,
        );
    }
    Ok(staged)
}

pub(crate) fn normalize_image(
    source: Vec<u8>,
    display_name: String,
    detail: hanafubuki_agent_kernel::AgentImageDetail,
) -> Result<hanafubuki_agent_kernel::PreparedAgentImage, String> {
    let reader = ImageReader::new(Cursor::new(&source))
        .with_guessed_format()
        .map_err(|error| format!("sniff image format: {error}"))?;
    let format = reader
        .format()
        .ok_or_else(|| "selected file is not a supported image".to_string())?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::WebP
    ) {
        return Err("selected image format is not supported".to_string());
    }
    let mut decoder = reader
        .into_decoder()
        .map_err(|error| format!("create image decoder: {error}"))?;
    let (width, height) = decoder.dimensions();
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| "image dimensions overflowed".to_string())?;
    if width == 0 || height == 0 || pixels > MAXIMUM_IMAGE_PIXELS {
        return Err(format!(
            "decoded image must contain at most {MAXIMUM_IMAGE_PIXELS} pixels"
        ));
    }
    let orientation = decoder
        .orientation()
        .map_err(|error| format!("read image orientation: {error}"))?;
    let mut image =
        DynamicImage::from_decoder(decoder).map_err(|error| format!("decode image: {error}"))?;
    image.apply_orientation(orientation);
    if width > MAXIMUM_IMAGE_DIMENSION || height > MAXIMUM_IMAGE_DIMENSION {
        image = image.resize(
            MAXIMUM_IMAGE_DIMENSION,
            MAXIMUM_IMAGE_DIMENSION,
            image::imageops::FilterType::Lanczos3,
        );
    }
    let (width, height) = image.dimensions();
    let mut normalized = Cursor::new(Vec::new());
    image
        .write_to(&mut normalized, ImageFormat::Png)
        .map_err(|error| format!("encode normalized image: {error}"))?;
    let mut bytes = normalized.into_inner();
    let mut mime_type = "image/png".to_string();
    if bytes.len() > MAXIMUM_NORMALIZED_IMAGE_BYTES {
        let rgb = image.to_rgb8();
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 85)
            .encode(rgb.as_raw(), width, height, image::ExtendedColorType::Rgb8)
            .map_err(|error| format!("encode bounded normalized image: {error}"))?;
        bytes = jpeg;
        mime_type = "image/jpeg".to_string();
    }
    if bytes.len() > MAXIMUM_NORMALIZED_IMAGE_BYTES {
        return Err(format!(
            "normalized image exceeds {MAXIMUM_NORMALIZED_IMAGE_BYTES} bytes"
        ));
    }
    Ok(hanafubuki_agent_kernel::PreparedAgentImage {
        bytes,
        mime_type,
        width,
        height,
        display_name,
        detail,
    })
}

#[cfg(test)]
mod tests;
