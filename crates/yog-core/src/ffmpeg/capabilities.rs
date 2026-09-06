use super::args::Arg;
use crate::{
    error::{Error, Failure},
    ffmpeg::Ffmpeg,
    process::run,
};
use std::io::Read;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncoderHelp {
    pub name: String,
    /// Absence means the help did not advertise a pixel-format list.
    pub pixel_formats: Option<Vec<String>>,
}

pub fn parse_encoder_help(text: &str) -> Result<EncoderHelp, Failure> {
    let name = text
        .lines()
        .find_map(|line| line.strip_prefix("Encoder ")?.split_whitespace().next())
        .ok_or(Failure::InvalidOutput(
            "FFmpeg encoder help has no encoder heading",
        ))?;
    let pixel_formats = text
        .lines()
        .find_map(|line| line.trim().strip_prefix("Supported pixel formats:"))
        .map(|formats| formats.split_whitespace().map(str::to_owned).collect());
    Ok(EncoderHelp {
        name: name.to_owned(),
        pixel_formats,
    })
}

impl Ffmpeg {
    pub fn encoder_help(&self, encoder: &str) -> Result<EncoderHelp, Error> {
        let mut args = Vec::new();
        for arg in [Arg::HideBanner, Arg::EncoderHelp(encoder)] {
            arg.append_to(&mut args);
        }
        let output = run(
            &self.program,
            &args,
            self.timeout,
            self.cancellation.clone(),
            |mut reader| {
                let mut text = String::new();
                reader.read_to_string(&mut text).map_err(Failure::Io)?;
                Ok(text)
            },
        )?;
        if !output.status.success() {
            return Err(output.failure(Failure::Exit));
        }
        let help = match parse_encoder_help(&output.value) {
            Ok(help) => help,
            Err(reason) => return Err(output.failure(reason)),
        };
        if help.name != encoder {
            return Err(output.failure(Failure::InvalidOutput(
                "FFmpeg help returned a different encoder",
            )));
        }
        Ok(help)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distinguishes_missing_pixel_formats_from_invalid_encoder() {
        let help = parse_encoder_help("Encoder ffv1 [FFmpeg video codec #1]:\n    Supported pixel formats: yuv420p yuv420p10le\n").unwrap();
        assert_eq!(help.pixel_formats.unwrap(), ["yuv420p", "yuv420p10le"]);
        assert!(
            parse_encoder_help("Encoder pcm_s16le [PCM]:\n")
                .unwrap()
                .pixel_formats
                .is_none()
        );
        assert!(parse_encoder_help("Codec 'missing' is not recognized by FFmpeg.").is_err());
    }
}
