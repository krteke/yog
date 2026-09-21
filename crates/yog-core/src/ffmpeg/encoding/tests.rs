use super::*;

#[test]
fn quality_parameter_names_match_the_ffmpeg_options() {
    assert_eq!(
        VideoEncoding::X264 {
            rate: None,
            preset: None,
        }
        .quality_parameter(),
        "CRF"
    );
    assert_eq!(
        VideoEncoding::SvtAv1 {
            rate: None,
            preset: None,
        }
        .quality_parameter(),
        "CRF"
    );
    assert_eq!(
        VideoEncoding::Rav1e {
            rate: None,
            speed: None,
        }
        .quality_parameter(),
        "QP"
    );
    assert_eq!(
        VideoEncoding::Nvenc {
            codec: VideoCodec::Av1,
            rate: None,
            preset: None,
            multipass: None,
        }
        .quality_parameter(),
        "CQ"
    );
    assert_eq!(
        VideoEncoding::Qsv {
            codec: VideoCodec::Hevc,
            rate: None,
            preset: None,
        }
        .quality_parameter(),
        "global_quality"
    );
    assert_eq!(
        VideoEncoding::Vaapi {
            codec: VideoCodec::H264,
            rate: None,
            device: PathBuf::new(),
        }
        .quality_parameter(),
        "QP"
    );
    assert_eq!(
        VideoEncoding::Vaapi {
            codec: VideoCodec::Av1,
            rate: None,
            device: PathBuf::new(),
        }
        .quality_parameter(),
        "global_quality"
    );
}

fn options(encoding: VideoEncoding) -> Vec<OsString> {
    let mut args = Vec::new();
    encoding.append_options(&mut args);
    args
}

#[test]
fn quality_controls_are_encoder_specific_and_bitrate_does_not_add_limits() {
    let quality = Some(RateControl::Quality(30));
    assert_eq!(
        options(VideoEncoding::X264 {
            rate: quality,
            preset: None,
        }),
        ["-crf:v", "30"].map(OsString::from)
    );
    assert_eq!(
        options(VideoEncoding::X265 {
            rate: quality,
            preset: None,
        }),
        ["-crf:v", "30"].map(OsString::from)
    );
    assert_eq!(
        options(VideoEncoding::SvtAv1 {
            rate: quality,
            preset: None,
        }),
        ["-crf:v", "30"].map(OsString::from)
    );
    assert_eq!(
        options(VideoEncoding::AomAv1 {
            rate: quality,
            cpu_used: None,
        }),
        ["-b:v", "0", "-crf:v", "30"].map(OsString::from)
    );
    assert_eq!(
        options(VideoEncoding::Rav1e {
            rate: quality,
            speed: None,
        }),
        ["-qp:v", "30"].map(OsString::from)
    );
    assert_eq!(
        options(VideoEncoding::Qsv {
            codec: VideoCodec::H264,
            rate: quality,
            preset: None,
        }),
        ["-global_quality:v", "30"].map(OsString::from)
    );
    assert_eq!(
        options(VideoEncoding::Vaapi {
            codec: VideoCodec::Hevc,
            rate: quality,
            device: "/dev/dri/renderD128".into(),
        }),
        ["-rc_mode:v", "CQP", "-qp:v", "30"].map(OsString::from)
    );
    assert_eq!(
        options(VideoEncoding::Vaapi {
            codec: VideoCodec::Av1,
            rate: quality,
            device: "/dev/dri/renderD128".into(),
        }),
        ["-rc_mode:v", "CQP", "-global_quality:v", "30"].map(OsString::from)
    );
    assert_eq!(
        options(VideoEncoding::Nvenc {
            codec: VideoCodec::Av1,
            rate: Some(RateControl::Bitrate(NonZeroU64::new(4_000_000).unwrap())),
            preset: None,
            multipass: None,
        }),
        ["-b:v", "4000000"].map(OsString::from)
    );
}
