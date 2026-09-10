use rustix::path::Arg;
use std::{collections::BTreeMap, fmt::Debug, path::Path};
use yog_core::{
    error::Error,
    ffprobe::{
        Ffprobe,
        types::{MediaInfo, MediaStream},
    },
};

pub trait Verifier {
    async fn verify(
        &self,
        input: &Path,
        output: &Path,
        original: &MediaInfo,
        copy_video: bool,
        warn: impl FnMut(String),
    ) -> Result<(), Error>;
}

impl Verifier for Ffprobe {
    async fn verify(
        &self,
        input: &Path,
        output: &Path,
        original: &MediaInfo,
        copy_video: bool,
        mut warn: impl FnMut(String),
    ) -> Result<(), Error> {
        let actual = self.probe(output).await?;
        if !actual.stderr.is_empty() {
            warn(format!(
                "output probe: {}",
                actual.stderr.to_string_lossy().trim()
            ));
        }
        let pairs = compare_structure(original, &actual.output, copy_video, &mut warn);
        if pairs.is_empty() {
            return Ok(());
        }

        let source_indices: Vec<_> = pairs.iter().map(|(before, _)| before.index).collect();
        let destination_indices: Vec<_> = pairs.iter().map(|(_, after)| after.index).collect();
        let source = self.packet_fingerprints(input, &source_indices).await?;
        let destination = self
            .packet_fingerprints(output, &destination_indices)
            .await?;

        for (label, stderr) in [("input", &source.stderr), ("output", &destination.stderr)] {
            if !stderr.is_empty() {
                warn(format!(
                    "{label} packet probe: {}",
                    stderr.to_string_lossy().trim()
                ));
            }
        }

        for (before, after) in pairs {
            let source = &source.output[&before.index];
            let destination = &destination.output[&after.index];
            let scope = format!(
                "{} stream #{} -> #{}",
                before.kind(),
                before.index,
                after.index
            );
            if source.sha256 != destination.sha256 || source.packets != destination.packets {
                warn(format!(
                    "{scope}: packet data/packetization differs (input {} packets, output {} packets)",
                    source.packets, destination.packets
                ));
            }
            if source.timing_sha256 != destination.timing_sha256 {
                warn(format!(
                    "{scope}: packet presentation times/durations differ"
                ));
            }
        }

        Ok(())
    }
}

fn compare_structure<'a>(
    original: &'a MediaInfo,
    actual: &'a MediaInfo,
    copy_video: bool,
    warn: &mut impl FnMut(String),
) -> Vec<(&'a MediaStream, &'a MediaStream)> {
    tags(
        "container",
        &original.format.tags,
        &actual.format.tags,
        warn,
    );
    changed(
        "container",
        "duration",
        &original.format.duration,
        &actual.format.duration,
        warn,
    );
    changed(
        "container",
        "program metadata",
        &original
            .programs
            .iter()
            .map(|p| p.get("tags"))
            .collect::<Vec<_>>(),
        &actual
            .programs
            .iter()
            .map(|p| p.get("tags"))
            .collect::<Vec<_>>(),
        warn,
    );

    changed(
        "chapters",
        "count",
        &original.chapters.len(),
        &actual.chapters.len(),
        warn,
    );
    for (index, (before, after)) in original.chapters.iter().zip(&actual.chapters).enumerate() {
        let scope = format!("chapter #{index}");
        changed(
            &scope,
            "start_time",
            &before.start_time,
            &after.start_time,
            warn,
        );
        changed(&scope, "end_time", &before.end_time, &after.end_time, warn);
        tags(&scope, &before.tags, &after.tags, warn);
    }

    let mut remaining: Vec<_> = actual.streams.iter().collect();
    let mut packets = Vec::new();

    for before in &original.streams {
        let Some(position) = remaining
            .iter()
            .position(|after| before.kind() == after.kind())
        else {
            warn(format!(
                "{} stream #{}: missing from output",
                before.kind(),
                before.index
            ));
            continue;
        };

        let after = remaining.remove(position);
        let scope = format!(
            "{} stream #{} -> #{}",
            before.kind(),
            before.index,
            after.index
        );
        tags(&scope, &before.tags, &after.tags, warn);

        let before_flags: BTreeMap<_, _> = before
            .disposition
            .iter()
            .filter(|(_, value)| **value != 0)
            .collect();
        let after_flags: BTreeMap<_, _> = after
            .disposition
            .iter()
            .filter(|(_, value)| **value != 0)
            .collect();

        changed(&scope, "disposition", &before_flags, &after_flags, warn);

        let mut before_side: Vec<_> = before.side_data_list.iter().collect();
        let mut after_side: Vec<_> = after.side_data_list.iter().collect();

        before_side.sort_by_key(|data| &data.side_data_type);
        after_side.sort_by_key(|data| &data.side_data_type);

        changed(&scope, "side data", &before_side, &after_side, warn);

        macro_rules! fields {
            ($($field:ident),+ $(,)?) => {{$(changed(&scope, stringify!($field), &before.$field, &after.$field, warn);)+}};
        }

        let copied = copy_video || !before.is_regular_video();
        if copied {
            fields!(codec_name, extradata_size, extradata_hash);
            if before.kind() == "attachment" {
                if before.extradata_hash.is_none() || after.extradata_hash.is_none() {
                    warn(format!(
                        "{scope}: cannot verify attachment contents; extradata hash unavailable"
                    ));
                }
            } else {
                packets.push((before, after));
            }
        }

        match before.codec_type.as_deref() {
            Some("audio") => fields!(
                sample_fmt,
                sample_rate,
                channels,
                channel_layout,
                bits_per_sample,
                bits_per_raw_sample
            ),
            Some("video") => {
                fields!(
                    width,
                    height,
                    sample_aspect_ratio,
                    color_range,
                    color_space,
                    color_transfer,
                    color_primaries,
                    chroma_location,
                    field_order
                );
                let source = original
                    .pixel_format(before)
                    .and_then(|format| format.samples());
                let destination = actual
                    .pixel_format(after)
                    .and_then(|format| format.samples());
                match (source, destination) {
                    (Some(source), Some(destination)) => {
                        changed(
                            &scope,
                            "component bit depths",
                            &source.component_depths,
                            &destination.component_depths,
                            warn,
                        );
                        changed(&scope, "chroma", &source.chroma, &destination.chroma, warn);
                        changed(&scope, "alpha", &source.alpha, &destination.alpha, warn);
                    }
                    _ => warn(format!(
                        "{scope}: cannot verify bit depth/chroma/alpha for pixel formats {:?} -> {:?}",
                        before.pix_fmt, after.pix_fmt
                    )),
                }
            }
            _ => {}
        }
    }

    for stream in remaining {
        warn(format!(
            "{} stream #{}: added in output",
            stream.kind(),
            stream.index
        ));
    }

    packets
}

fn changed<T: Debug + PartialEq>(
    scope: &str,
    field: &str,
    before: &T,
    after: &T,
    warn: &mut impl FnMut(String),
) {
    if before != after {
        warn(format!("{scope}, {field}: {before:?} -> {after:?}"));
    }
}

fn tags(
    scope: &str,
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
    warn: &mut impl FnMut(String),
) {
    for (key, value) in before {
        let actual = after
            .iter()
            .find(|(other, _)| key.eq_ignore_ascii_case(other))
            .map(|(_, value)| value);
        changed(
            scope,
            &format!("metadata {key:?}"),
            &Some(value),
            &actual,
            warn,
        );
    }

    for (key, value) in after {
        if !before.keys().any(|other| key.eq_ignore_ascii_case(other)) {
            warn(format!("{scope}, metadata {key:?}: added {value:?}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn media(value: serde_json::Value) -> MediaInfo {
        let mut media: MediaInfo = serde_json::from_value(value).unwrap();
        media.pixel_formats = serde_json::from_str::<MediaInfo>(include_str!(
            "../../yog-core/src/ffmpeg/test_pixel_formats.json"
        ))
        .unwrap()
        .pixel_formats;
        media
    }

    #[test]
    fn matching_handles_renumbering_cover_relocation_and_equivalent_pixel_layouts() {
        let before = media(json!({
            "streams": [
                {"index": 9, "codec_type": "video", "codec_name": "mjpeg", "pix_fmt":"yuv420p", "disposition":{"attached_pic":1}},
                {"index": 5, "codec_type": "audio", "codec_name":"aac", "tags":{"language":"jpn"}},
                {"index": 11, "codec_type":"video", "codec_name":"hevc", "pix_fmt":"yuv420p10le"},
                {"index": 15, "codec_type":"audio", "codec_name":"flac", "tags":{"language":"eng"}}
            ],
            "format":{"tags":{"title":"unchanged"}},
            "chapters":[{"id":10,"time_base":"1/1000","start":500,"start_time":"0.500000","end_time":"1.000000"}]
        }));
        let after = media(json!({
            "streams": [
                {"index": 0, "codec_type": "audio", "codec_name":"aac", "tags":{"LANGUAGE":"jpn"}, "disposition":{"default":0}},
                {"index": 1, "codec_type":"video", "codec_name":"h264", "pix_fmt":"p010le"},
                {"index": 2, "codec_type":"audio", "codec_name":"flac", "tags":{"language":"eng"}},
                {"index": 3, "codec_type": "video", "codec_name": "mjpeg", "pix_fmt":"yuv420p", "disposition":{"attached_pic":1}}
            ],
            "format":{"tags":{"TITLE":"unchanged"}},
            "chapters":[{"id":0,"time_base":"1/1000000","start":500000,"start_time":"0.500000","end_time":"1.000000"}]
        }));
        let mut warnings = Vec::new();
        let pairs = compare_structure(&before, &after, false, &mut |warning| {
            warnings.push(warning)
        });
        assert!(warnings.is_empty(), "{warnings:#?}");
        assert_eq!(
            pairs
                .iter()
                .map(|(a, b)| (a.index, b.index))
                .collect::<Vec<_>>(),
            [(9, 3), (5, 0), (15, 2)]
        );
        compare_structure(&before, &after, true, &mut |warning| warnings.push(warning));
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("codec_name"))
        );
    }

    #[test]
    fn lost_streams_metadata_chapters_and_sample_changes_are_reported_individually() {
        let before = media(json!({
            "streams":[
                {"index":0,"codec_type":"video","pix_fmt":"yuv444p12le","color_transfer":"smpte2084"},
                {"index":1,"codec_type":"subtitle","tags":{"language":"eng"}},
                {"index":2,"codec_type":"attachment","extradata_size":3,"extradata_hash":"SHA256:before"},
                {"index":3,"codec_type":"video","pix_fmt":"yuv420p","disposition":{"attached_pic":1}},
                {"index":4,"codec_type":"audio","channels":2,"sample_rate":"48000","tags":{"title":"commentary"}}
            ],
            "format":{"tags":{"title":"original","ENCODER":"old"}},
            "chapters":[{"start_time":"0.000000","end_time":"1.000000","tags":{"title":"chapter"}},{}]
        }));
        let after = media(json!({
            "streams":[
                {"index":0,"codec_type":"video","pix_fmt":"yuv420p"},
                {"index":1,"codec_type":"attachment","extradata_size":3,"extradata_hash":"SHA256:after"},
                {"index":2,"codec_type":"audio","channels":1,"sample_rate":"44100","tags":{"title":"main"}},
                {"index":3,"codec_type":"data"}
            ],
            "format":{"tags":{"encoder":"new","comment":"added"}},
            "chapters":[{"start_time":"0.000000","end_time":"0.500000","tags":{}}]
        }));
        let mut warnings = Vec::new();
        compare_structure(&before, &after, false, &mut |warning| {
            warnings.push(warning)
        });
        for expected in [
            "subtitle stream #1: missing",
            "cover stream #3: missing",
            "data stream #3: added",
            "component bit depths",
            "chroma",
            "color_transfer",
            "extradata_hash",
            "channels",
            "sample_rate",
            "chapters, count",
            "chapter #0, end_time",
            "metadata \"title\"",
            "metadata \"ENCODER\"",
            "metadata \"comment\"",
        ] {
            assert!(
                warnings.iter().any(|warning| warning.contains(expected)),
                "{expected}: {warnings:#?}"
            );
        }
        let unknown = media(json!({"streams":[{"index":0,"codec_type":"video","pix_fmt":"pal8"}]}));
        warnings.clear();
        compare_structure(&unknown, &unknown, false, &mut |warning| {
            warnings.push(warning)
        });
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("cannot verify bit depth"))
        );
    }
}
