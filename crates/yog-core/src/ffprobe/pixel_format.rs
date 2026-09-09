use super::types::{MediaInfo, MediaStream, PixelFormat};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoSamples {
    pub component_depths: Vec<u8>,
    pub chroma: Chroma,
    pub alpha: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chroma {
    Yuv { log2_width: u8, log2_height: u8 },
    Rgb,
    Gray,
}

impl MediaInfo {
    pub fn pixel_format(&self, stream: &MediaStream) -> Option<&PixelFormat> {
        let name = stream.pix_fmt.as_deref()?;
        self.pixel_formats.iter().find(|format| format.name == name)
    }
}

impl PixelFormat {
    pub fn samples(&self) -> Option<VideoSamples> {
        if self.flags.hwaccel != 0 || self.flags.palette != 0 {
            return None;
        }
        let alpha = self.flags.alpha != 0;
        let chroma = match self.nb_components - u8::from(alpha) {
            1 => Chroma::Gray,
            3 if self.flags.rgb != 0 => Chroma::Rgb,
            3 => Chroma::Yuv {
                log2_width: self.log2_chroma_w?,
                log2_height: self.log2_chroma_h?,
            },
            _ => return None,
        };
        Some(VideoSamples {
            component_depths: self.components.iter().map(|c| c.bit_depth).collect(),
            chroma,
            alpha,
        })
    }
}
