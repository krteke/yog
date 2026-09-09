use crate::ffprobe::{
    pixel_format::{Chroma, VideoSamples},
    types::{MediaInfo, PixelFormat},
};

pub(super) fn encoder_format<'a>(
    media: &'a MediaInfo,
    input: &'a PixelFormat,
    supported: &[String],
) -> &'a str {
    if supported.contains(&input.name) {
        return &input.name;
    }
    let Some(samples) = input.samples() else {
        return &input.name;
    };
    supported
        .iter()
        .find_map(|name| {
            media.pixel_formats.iter().find(|candidate| {
                candidate.name == *name && candidate.samples().as_ref() == Some(&samples)
            })
        })
        .map_or(&input.name, |format| &format.name)
}

pub(super) fn vaapi_qsv_format(input: &PixelFormat) -> &str {
    let Some(VideoSamples {
        component_depths,
        chroma,
        alpha: false,
    }) = input.samples()
    else {
        return &input.name;
    };

    let (width, height) = match chroma {
        Chroma::Yuv {
            log2_width,
            log2_height,
        } => (log2_width, log2_height),
        _ => return &input.name,
    };

    match (width, height, component_depths.as_slice()) {
        (1, 1, [8, 8, 8]) => "nv12",
        (1, 1, [10, 10, 10]) => "p010le",
        (1, 1, [12, 12, 12]) => "p012le",
        (1, 0, [8, 8, 8]) => "yuyv422",
        (1, 0, [10, 10, 10]) => "y210le",
        (1, 0, [12, 12, 12]) => "y212le",
        (0, 0, [8, 8, 8]) => "vuyx",
        (0, 0, [10, 10, 10]) => "xv30le",
        (0, 0, [12, 12, 12]) => "xv36le",
        _ => &input.name,
    }
}
